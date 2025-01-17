use std::{path::Path, process::Stdio, sync::Arc, time::Duration};

use compositor_common::{
    frame::YuvData,
    renderer_spec::{FallbackStrategy, RendererId, RendererSpec, ShaderSpec},
    scene::{constraints::NodeConstraints, NodeId, NodeSpec, OutputSpec, Resolution, SceneSpec},
    Frame, Framerate,
};
use compositor_render::{renderer::RendererOptions, FrameSet, Renderer, WebRendererOptions};

const FRAMERATE: Framerate = Framerate { num: 30, den: 1 };

fn ffmpeg_yuv_to_jpeg(
    input_file: impl AsRef<Path>,
    output_file: impl AsRef<Path>,
    resolution: Resolution,
) {
    std::process::Command::new("ffmpeg")
        .arg("-s")
        .arg(format!("{}x{}", resolution.width, resolution.height))
        .arg("-pix_fmt")
        .arg("yuv420p")
        .arg("-i")
        .arg(input_file.as_ref().as_os_str())
        .arg(output_file.as_ref().as_os_str())
        .arg("-y")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn ffmpeg")
        .wait()
        .expect("wait");
}

fn ffmpeg_jpeg_to_yuv(input_file: impl AsRef<Path>, output_file: impl AsRef<Path>) {
    std::process::Command::new("ffmpeg")
        .arg("-i")
        .arg(input_file.as_ref().as_os_str())
        .arg(output_file.as_ref().as_os_str())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
        .wait()
        .unwrap();
}

fn get_image(path: impl AsRef<Path>) -> Frame {
    ffmpeg_jpeg_to_yuv(&path, "input.yuv");

    let jpeg = image::open(path).expect("picture load");
    let resolution = Resolution {
        width: jpeg.width() as usize,
        height: jpeg.height() as usize,
    };

    let yuv = std::fs::read("input.yuv").expect("yuv load");
    let yuv = bytes::Bytes::from(yuv);

    let y_len = resolution.width * resolution.height;
    let yuv_data = YuvData {
        y_plane: yuv.slice(0..y_len),
        u_plane: yuv.slice(y_len..5 * y_len / 4),
        v_plane: yuv.slice(5 * y_len / 4..),
    };
    assert_eq!(yuv_data.u_plane.len(), yuv_data.v_plane.len());

    std::fs::remove_file("input.yuv").expect("rm input.yuv");

    Frame {
        data: yuv_data,
        pts: Duration::from_secs(1),
        resolution,
    }
}

fn main() {
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    let frame = get_image("./examples/silly/crab.jpg");
    let resolution = frame.resolution;

    let (mut renderer, _) = Renderer::new(RendererOptions {
        web_renderer: WebRendererOptions {
            init: false,
            ..Default::default()
        },
        framerate: FRAMERATE,
        stream_fallback_timeout: Duration::from_secs(1),
    })
    .expect("create renderer");
    let shader_key = RendererId("silly shader".into());

    renderer
        .register_renderer(RendererSpec::Shader(ShaderSpec {
            shader_id: shader_key.clone(),
            source: include_str!("./silly/silly.wgsl").into(),
            fallback_strategy: FallbackStrategy::FallbackIfAllInputsMissing,
            constraints: NodeConstraints::empty(),
        }))
        .expect("create shader");

    let input_id = NodeId("input".into());
    let shader_id = NodeId("silly".into());
    let output_id = NodeId("output".into());

    renderer
        .update_scene(Arc::new(SceneSpec {
            nodes: vec![NodeSpec {
                input_pads: vec![input_id.clone()],
                node_id: shader_id.clone(),
                params: compositor_common::scene::NodeParams::Shader {
                    shader_id: shader_key,
                    shader_params: None,
                    resolution,
                },
                fallback_id: None,
            }],
            outputs: vec![OutputSpec {
                input_pad: shader_id,
                output_id: output_id.clone().into(),
            }],
        }))
        .expect("update scene");

    let mut frame_set = FrameSet::new(Duration::from_secs_f32(std::f32::consts::FRAC_PI_2));
    frame_set.frames.insert(input_id.into(), frame);
    let output = renderer.render(frame_set).expect("render");
    let output = output.frames.get(&output_id.into()).expect("extract frame");
    let mut output_data = Vec::with_capacity(resolution.width * resolution.height * 3 / 2);
    output_data.extend_from_slice(&output.data.y_plane);
    output_data.extend_from_slice(&output.data.u_plane);
    output_data.extend_from_slice(&output.data.v_plane);
    std::fs::write("output.yuv", output_data).expect("write");

    ffmpeg_yuv_to_jpeg("output.yuv", "output.jpg", resolution);

    std::fs::remove_file("output.yuv").expect("rm output.yuv");
}
