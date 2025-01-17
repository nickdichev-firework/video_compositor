# VideoCompositor

Application for real-time video processing/transforming/composing, providing simple, language-agnostic API for live video rendering.

VideoCompositor targets real-time use cases, like video conferencing, live-streaming, or broadcasting (e.g. with [WebRTC](https://en.wikipedia.org/wiki/WebRTC) / [HLS](https://en.wikipedia.org/wiki/HTTP_Live_Streaming) / [RTMP](https://en.wikipedia.org/wiki/Real-Time_Messaging_Protocol)).

## Features

VideoCompositor receives inputs and sends outputs streams via [RTP](https://en.wikipedia.org/wiki/Real-time_Transport_Protocol).
HTTP API is used to define how inputs should be transformed and combined to produce desired outputs.

For the initial release, we want VideoCompositor to support those four types of transformations, that you can combine together:

- Common transformations - frequently used, already implemented transformations, like layouts, grids, cropping, corners rounding, blending, fading, etc.
- Custom shader transformations - registering and using custom shaders, allowing to adapt VideoCompositor for specific business needs
- Web Rendering - embedding videos in custom websites
- Text Rendering

Currently, the project is under development and API is unstable.

## Examples

Examples source code is under the `examples` directory.

Running examples requires:

- [Rust](https://www.rust-lang.org/tools/install)
- [FFmpeg 6.0](https://ffmpeg.org/download.html)

For better performance, build examples with the [release compilation profile](https://doc.rust-lang.org/book/ch14-01-release-profiles.html):

```console
cargo run --release --example <example_name>
```

You can also check out [RTC.ON 2023 workshops repo](https://github.com/membraneframework-labs/rtcon_video_compositor_workshops) for more examples / exercises.

## Supported platforms

Linux and MacOS.

## Copyright

Copyright 2023, [Software Mansion](https://swmansion.com/?utm_source=git&utm_medium=readme&utm_campaign=video_compositor)

[![Software Mansion](https://logo.swmansion.com/logo?color=white&variant=desktop&width=200&tag=membrane-github)](https://swmansion.com/?utm_source=git&utm_medium=readme&utm_campaign=video_compositor)
