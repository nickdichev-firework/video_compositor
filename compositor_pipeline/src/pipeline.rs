use std::collections::{hash_map, HashMap};
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use compositor_common::error::ErrorStack;
use compositor_common::renderer_spec::{RendererId, RendererSpec};
use compositor_common::scene::{InputId, OutputId, Resolution, SceneSpec};
use compositor_common::Framerate;
use compositor_render::error::{
    InitRendererEngineError, RegisterRendererError, UnregisterRendererError,
};
use compositor_render::renderer::RendererOptions;
use compositor_render::EventLoop;
use compositor_render::{error::UpdateSceneError, Renderer};
use compositor_render::{RegistryType, WebRendererOptions};
use crossbeam_channel::unbounded;
use ffmpeg_next::Packet;
use log::{error, warn};

use crate::error::{
    RegisterInputError, RegisterOutputError, UnregisterInputError, UnregisterOutputError,
};
use crate::queue::Queue;

use self::decoder::Decoder;
use self::encoder::{Encoder, EncoderSettings};

pub mod decoder;
pub mod encoder;

pub trait PipelineOutput: Send + Sync + Sized + Clone + 'static {
    type Opts: Send + Sync + 'static;
    type Context: 'static;

    fn send_packet(&self, context: &mut Self::Context, packet: Packet);
    fn new(
        opts: Self::Opts,
        codec: ffmpeg_next::Codec,
    ) -> Result<(Self, Self::Context), Box<dyn std::error::Error + Send + Sync + 'static>>;
}

pub trait PipelineInput: Send + Sync + Sized + 'static {
    type Opts: Send + Sync;
    type PacketIterator: Iterator<Item = Packet> + Send;

    fn new(opts: Self::Opts) -> (Self, Self::PacketIterator);
    fn decoder_parameters(&self) -> decoder::DecoderParameters;
}

pub struct OutputOptions<Output: PipelineOutput> {
    pub receiver_options: Output::Opts,
    pub encoder_settings: EncoderSettings,
    pub resolution: Resolution,
}

pub struct Pipeline<Input: PipelineInput, Output: PipelineOutput> {
    inputs: HashMap<InputId, Arc<Decoder<Input>>>,
    outputs: OutputRegistry<Encoder<Output>>,
    queue: Arc<Queue>,
    renderer: Renderer,
    is_started: bool,
}

pub struct Options {
    pub framerate: Framerate,
    pub stream_fallback_timeout: Duration,
    pub web_renderer: WebRendererOptions,
}

impl<Input: PipelineInput, Output: PipelineOutput> Pipeline<Input, Output> {
    pub fn new(opts: Options) -> Result<(Self, EventLoop), InitRendererEngineError> {
        let (renderer, event_loop) = Renderer::new(RendererOptions {
            web_renderer: opts.web_renderer,
            framerate: opts.framerate,
            stream_fallback_timeout: opts.stream_fallback_timeout,
        })?;
        let pipeline = Pipeline {
            outputs: OutputRegistry::new(),
            inputs: HashMap::new(),
            queue: Arc::new(Queue::new(opts.framerate)),
            renderer,
            is_started: false,
        };

        Ok((pipeline, event_loop))
    }

    pub fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn register_input(
        &mut self,
        input_id: InputId,
        input_opts: Input::Opts,
    ) -> Result<(), RegisterInputError> {
        if self.inputs.contains_key(&input_id) {
            return Err(RegisterInputError::AlreadyRegistered(input_id));
        }

        self.inputs.insert(
            input_id.clone(),
            Decoder::new(self.queue.clone(), input_opts, input_id.clone()).into(),
        );
        self.queue.add_input(input_id);
        Ok(())
    }

    pub fn unregister_input(&mut self, input_id: &InputId) -> Result<(), UnregisterInputError> {
        if !self.inputs.contains_key(input_id) {
            return Err(UnregisterInputError::NotFound(input_id.clone()));
        }

        let scene_spec = self.renderer.scene_spec();
        let is_still_in_use = scene_spec
            .nodes
            .iter()
            .flat_map(|node| &node.input_pads)
            .any(|input_pad_id| &input_id.0 == input_pad_id);
        if is_still_in_use {
            return Err(UnregisterInputError::StillInUse(input_id.clone()));
        }

        self.inputs.remove(input_id);
        self.queue.remove_input(input_id);
        Ok(())
    }

    pub fn register_output(
        &self,
        output_id: OutputId,
        output_opts: OutputOptions<Output>,
    ) -> Result<(), RegisterOutputError> {
        if self.outputs.contains_key(&output_id) {
            return Err(RegisterOutputError::AlreadyRegistered(output_id));
        }

        if output_opts.resolution.height % 2 != 0 || output_opts.resolution.width % 2 != 0 {
            return Err(RegisterOutputError::UnsupportedResolution(output_id));
        }

        let output = Encoder::new(output_opts)
            .map_err(|e| RegisterOutputError::EncoderError(output_id.clone(), e))?;

        self.outputs.insert(output_id, output.into());
        Ok(())
    }

    pub fn unregister_output(&self, output_id: &OutputId) -> Result<(), UnregisterOutputError> {
        if !self.outputs.contains_key(output_id) {
            return Err(UnregisterOutputError::NotFound(output_id.clone()));
        }

        let scene_spec = self.renderer.scene_spec();
        let is_still_in_use = scene_spec
            .outputs
            .iter()
            .any(|node| &node.output_id == output_id);
        if is_still_in_use {
            return Err(UnregisterOutputError::StillInUse(output_id.clone()));
        }

        self.outputs.remove(output_id);
        Ok(())
    }

    pub fn register_renderer(
        &self,
        transformation_spec: RendererSpec,
    ) -> Result<(), RegisterRendererError> {
        self.renderer.register_renderer(transformation_spec)?;
        Ok(())
    }

    pub fn unregister_renderer(
        &self,
        renderer_id: &RendererId,
        registry_type: RegistryType,
    ) -> Result<(), UnregisterRendererError> {
        self.renderer
            .unregister_renderer(renderer_id, registry_type)
    }

    pub fn update_scene(&mut self, scene_spec: Arc<SceneSpec>) -> Result<(), UpdateSceneError> {
        scene_spec
            .validate(
                &self.inputs.keys().map(|i| &i.0).collect(),
                &self.outputs.lock().keys().map(|i| &i.0).collect(),
            )
            .map_err(UpdateSceneError::InvalidSpec)?;
        self.renderer.update_scene(scene_spec)
    }

    pub fn start(&mut self) {
        if self.is_started {
            error!("Pipeline already started.");
            return;
        }
        let (frames_sender, frames_receiver) = unbounded();
        let renderer = self.renderer.clone();
        let outputs = self.outputs.clone();

        self.queue.start(frames_sender);

        thread::spawn(move || {
            for input_frames in frames_receiver.iter() {
                if frames_receiver.len() > 20 {
                    warn!("Dropping frame: render queue is too long.",);
                    continue;
                }

                let output = renderer.render(input_frames);
                let Ok(output_frames) = output else {
                    error!(
                        "Error while rendering: {}",
                        ErrorStack::new(&output.unwrap_err()).into_string()
                    );
                    continue;
                };

                for (id, frame) in output_frames.frames {
                    let output = outputs.lock().get(&id).map(Clone::clone);
                    let Some(output) = output else {
                        error!("no output with id {}", &id);
                        continue;
                    };

                    output.send_frame(frame);
                }
            }
        });
    }

    pub fn inputs(&self) -> impl Iterator<Item = (&InputId, &Input)> {
        self.inputs.iter().map(|(id, node)| (id, node.input()))
    }

    pub fn with_outputs<F, R>(&self, f: F) -> R
    where
        F: Fn(OutputIterator<'_, Output>) -> R,
    {
        let guard = self.outputs.lock();
        f(OutputIterator::new(guard.iter()))
    }
}

struct OutputRegistry<T>(Arc<Mutex<HashMap<OutputId, Arc<T>>>>);

impl<T> Clone for OutputRegistry<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> OutputRegistry<T> {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(HashMap::new())))
    }

    fn contains_key(&self, key: &OutputId) -> bool {
        self.0.lock().unwrap().contains_key(key)
    }

    fn insert(&self, key: OutputId, value: Arc<T>) -> Option<Arc<T>> {
        self.0.lock().unwrap().insert(key, value)
    }

    fn remove(&self, key: &OutputId) -> Option<Arc<T>> {
        self.0.lock().unwrap().remove(key)
    }

    fn lock(&self) -> MutexGuard<HashMap<OutputId, Arc<T>>> {
        self.0.lock().unwrap()
    }
}

pub struct OutputIterator<'a, Output: PipelineOutput> {
    inner_iter: hash_map::Iter<'a, OutputId, Arc<Encoder<Output>>>,
}

impl<'a, Output: PipelineOutput> OutputIterator<'a, Output> {
    fn new(iter: hash_map::Iter<'a, OutputId, Arc<Encoder<Output>>>) -> Self {
        Self { inner_iter: iter }
    }
}

impl<'a, Output: PipelineOutput> Iterator for OutputIterator<'a, Output> {
    type Item = (&'a OutputId, &'a Output);

    fn next(&mut self) -> Option<Self::Item> {
        self.inner_iter.next().map(|(id, node)| (id, node.output()))
    }
}
