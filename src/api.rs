use std::sync::Arc;

use compositor_pipeline::pipeline::{self};
use compositor_render::{EventLoop, RegistryType};
use crossbeam_channel::{bounded, Receiver};

use serde::{Deserialize, Serialize};
use tiny_http::StatusCode;

use crate::{
    error::ApiError,
    rtp_receiver::{self, RtpReceiver},
    rtp_sender::{self, RtpSender},
    types::{
        self, InitOptions, InputId, OutputId, RegisterInputRequest, RegisterOutputRequest,
        RegisterRequest, RendererId, Scene,
    },
};

pub type Pipeline = compositor_pipeline::Pipeline<RtpReceiver, RtpSender>;

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Init(InitOptions),
    Register(RegisterRequest),
    Unregister(UnregisterRequest),
    UpdateScene(types::Scene),
    Query(QueryRequest),
    Start,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "entity_type", rename_all = "snake_case")]
pub enum UnregisterRequest {
    InputStream { input_id: InputId },
    OutputStream { output_id: OutputId },
    Shader { shader_id: RendererId },
    WebRenderer { instance_id: RendererId },
    Image { image_id: RendererId },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "query", rename_all = "snake_case")]
pub enum QueryRequest {
    WaitForNextFrame { input_id: InputId },
    Scene,
    Inputs,
    Outputs,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    Ok {},
    Scene(Scene),
    Inputs { inputs: Vec<InputInfo> },
    Outputs { outputs: Vec<OutputInfo> },
}

#[derive(Serialize, Deserialize)]
pub struct InputInfo {
    pub id: InputId,
    pub port: u16,
}

#[derive(Serialize, Deserialize)]
pub struct OutputInfo {
    pub id: OutputId,
    pub port: u16,
    pub ip: Arc<str>,
}

pub enum ResponseHandler {
    Response(Response),
    Ok,
    DeferredResponse(Receiver<Result<Response, ApiError>>),
}

pub struct Api {
    pipeline: Pipeline,
}

impl Api {
    pub fn new(opts: pipeline::Options) -> Result<(Api, EventLoop), ApiError> {
        let (pipeline, event_loop) = Pipeline::new(opts)?;
        Ok((Api { pipeline }, event_loop))
    }

    pub fn handle_request(&mut self, request: Request) -> Result<ResponseHandler, ApiError> {
        match request {
            Request::Init(_) => Err(ApiError::new(
                "COMPOSITOR_ALREADY_INITIALIZED",
                "Compositor was already initialized.".to_string(),
                StatusCode(400),
            )),
            Request::Register(register_request) => {
                self.handle_register_request(register_request)?;
                Ok(ResponseHandler::Ok)
            }
            Request::Unregister(unregister_request) => {
                self.handle_unregister_request(unregister_request)?;
                Ok(ResponseHandler::Ok)
            }
            Request::Start => {
                self.pipeline.start();
                Ok(ResponseHandler::Ok)
            }
            Request::UpdateScene(scene_spec) => {
                self.pipeline
                    .update_scene(Arc::new(scene_spec.try_into()?))?;
                Ok(ResponseHandler::Ok)
            }
            Request::Query(query) => self.handle_query(query),
        }
    }

    fn handle_query(&self, query: QueryRequest) -> Result<ResponseHandler, ApiError> {
        match query {
            QueryRequest::WaitForNextFrame { input_id } => {
                let (sender, receiver) = bounded(1);
                self.pipeline.queue().subscribe_input_listener(
                    input_id.into(),
                    Box::new(move || {
                        sender.send(Ok(Response::Ok {})).unwrap();
                    }),
                );
                Ok(ResponseHandler::DeferredResponse(receiver))
            }
            QueryRequest::Scene => Ok(ResponseHandler::Response(Response::Scene(
                self.pipeline
                    .renderer()
                    .scene_spec()
                    .as_ref()
                    .clone()
                    .into(),
            ))),
            QueryRequest::Inputs => {
                let inputs = self
                    .pipeline
                    .inputs()
                    .map(|(id, node)| InputInfo {
                        id: id.clone().into(),
                        port: node.port,
                    })
                    .collect();
                Ok(ResponseHandler::Response(Response::Inputs { inputs }))
            }
            QueryRequest::Outputs => {
                let outputs = self.pipeline.with_outputs(|iter| {
                    iter.map(|(id, output)| OutputInfo {
                        id: id.clone().into(),
                        port: output.port,
                        ip: output.ip.clone(),
                    })
                    .collect()
                });
                Ok(ResponseHandler::Response(Response::Outputs { outputs }))
            }
        }
    }

    fn handle_register_request(&mut self, request: RegisterRequest) -> Result<(), ApiError> {
        match request {
            RegisterRequest::InputStream(input_stream) => self.register_input(input_stream),
            RegisterRequest::OutputStream(output_stream) => self.register_output(output_stream),
            RegisterRequest::Shader(spec) => {
                let spec = spec.try_into()?;
                Ok(self.pipeline.register_renderer(spec)?)
            }
            RegisterRequest::WebRenderer(spec) => {
                let spec = spec.try_into()?;
                Ok(self.pipeline.register_renderer(spec)?)
            }
            RegisterRequest::Image(spec) => {
                let spec = spec.try_into()?;
                Ok(self.pipeline.register_renderer(spec)?)
            }
        }
    }

    fn handle_unregister_request(&mut self, request: UnregisterRequest) -> Result<(), ApiError> {
        match request {
            UnregisterRequest::InputStream { input_id } => {
                Ok(self.pipeline.unregister_input(&input_id.into())?)
            }
            UnregisterRequest::OutputStream { output_id } => {
                Ok(self.pipeline.unregister_output(&output_id.into())?)
            }
            UnregisterRequest::Shader { shader_id } => Ok(self
                .pipeline
                .unregister_renderer(&shader_id.into(), RegistryType::Shader)?),
            UnregisterRequest::WebRenderer { instance_id } => Ok(self
                .pipeline
                .unregister_renderer(&instance_id.into(), RegistryType::WebRenderer)?),
            UnregisterRequest::Image { image_id } => Ok(self
                .pipeline
                .unregister_renderer(&image_id.into(), RegistryType::Image)?),
        }
    }

    fn register_output(&mut self, request: RegisterOutputRequest) -> Result<(), ApiError> {
        let RegisterOutputRequest {
            output_id,
            port,
            resolution,
            encoder_settings,
            ip,
        } = request;

        self.pipeline.with_outputs(|mut iter| {
            if let Some((node_id, _)) = iter.find(|(_, output)| output.port == port && output.ip == ip) {
                return Err(ApiError::new(
                    "PORT_AND_IP_ALREADY_IN_USE",
                    format!("Failed to register output stream \"{output_id}\". Combination of port {port} and IP {ip} is already used by node \"{node_id}\""),
                    tiny_http::StatusCode(400)
                ));
            };
            Ok(())
        })?;

        self.pipeline.register_output(
            output_id.into(),
            pipeline::OutputOptions {
                resolution: resolution.into(),
                encoder_settings: encoder_settings.into(),
                receiver_options: rtp_sender::Options { port, ip },
            },
        )?;

        Ok(())
    }

    fn register_input(&mut self, request: RegisterInputRequest) -> Result<(), ApiError> {
        let RegisterInputRequest { input_id: id, port } = request;

        if let Some((node_id, _)) = self.pipeline.inputs().find(|(_, input)| input.port == port) {
            return Err(ApiError::new(
                "PORT_ALREADY_IN_USE",
                format!("Failed to register input stream \"{id}\". Port {port} is already used by node \"{node_id}\""),
                tiny_http::StatusCode(400)
            ));
        }

        self.pipeline.register_input(
            id.clone().into(),
            rtp_receiver::Options {
                port,
                input_id: id.into(),
            },
        )?;

        Ok(())
    }
}
