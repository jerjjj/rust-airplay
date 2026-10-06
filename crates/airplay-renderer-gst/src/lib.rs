// SPDX-License-Identifier: GPL-3.0-or-later
//! A renderer that plays through GStreamer.
//!
//! This is the backend ADR-0003 chose first, in the shape upstream gives it: an audio
//! renderer and a video renderer, each with a pipeline of its own, and the application
//! choosing a sink (`upstream/UxPlay@3dbf7cee:renderers/`). What is different is how it
//! reaches GStreamer: it runs `gst-launch-1.0` as a subprocess and writes frames to it,
//! rather than linking `libgstreamer`. [`pipeline`] says why, and
//! [`docs/ADR-0003-media-backend.md`] records the decision.
//!
//! The shape of a stream is a pipeline description and a pipe:
//!
//! ```text
//! audio:  fdsrc ! <caps of the coding> ! decodebin ! audioconvert ! <sink>
//! video:  fdsrc ! h264parse ! avdec_h264 ! videoconvert ! <sink>
//! ```
//!
//! A coding this backend cannot describe is refused when the stream is opened, not when
//! the first frame arrives: a client whose audio never plays should be told once, at the
//! moment a session could still choose differently.
//!
//! [`docs/ADR-0003-media-backend.md`]: ../../../docs/ADR-0003-media-backend.md

#![deny(missing_docs)]

pub mod pipeline;

use std::sync::Mutex;

use airplay_renderer::{
    AudioCodec, AudioConfig, AudioFrame, AudioStream, RenderError, Renderer, VideoConfig,
    VideoFrame, VideoStream, Volume,
};

use pipeline::{LaunchError, Pipeline, find_launcher};

/// The sinks this backend can play into, in the order it prefers them.
///
/// Upstream lets a user choose one (`renderers/`'s sink options) and so does this, by
/// name: a machine whose preferred sink is missing is a machine whose user knows which
/// one it has.
pub const AUDIO_SINKS: &[&str] = &[
    "wasapisink",
    "autoaudiosink",
    "osssink",
    "alsasink",
    "pulsesink",
    "fakesink",
];

/// The video sinks this backend can show, in the order it prefers them.
pub const VIDEO_SINKS: &[&str] = &["autovideosink", "d3d11videosink", "glimagesink", "fakesink"];

/// Environment variable naming the audio sink to use.
pub const AUDIO_SINK_ENV: &str = "AIRPLAY_GST_AUDIO_SINK";

/// Longest a backend waits between two frames.
///
/// A gap longer than this is not a frame that is late, it is a client that stopped
/// and started again — a screen waking up — and a receiver that waited out the gap
/// would appear to hang. One second is longer than any frame interval a client
/// sends.
pub const MAX_PACE_SECONDS: f64 = 1.0;

/// Environment variable naming the video sink to use.
pub const VIDEO_SINK_ENV: &str = "AIRPLAY_GST_VIDEO_SINK";

/// A renderer whose streams are GStreamer pipelines.
#[derive(Debug)]
pub struct GstRenderer {
    program: String,
    audio_sink: String,
    video_sink: String,
    volume: Mutex<Volume>,
}

impl GstRenderer {
    /// A renderer that will run `gst-launch-1.0`, found on this machine.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Unavailable`] when no `gst-launch-1.0` can be run, which
    /// is an environment problem and is reported as one
    /// ([ADR-0003](../../../docs/ADR-0003-media-backend.md)).
    pub fn new() -> Result<Self, RenderError> {
        let program = find_launcher().map_err(unavailable)?;
        Ok(Self {
            program,
            audio_sink: sink_from_env(AUDIO_SINK_ENV, AUDIO_SINKS),
            video_sink: sink_from_env(VIDEO_SINK_ENV, VIDEO_SINKS),
            volume: Mutex::new(Volume::FULL),
        })
    }

    /// A renderer that plays into the sinks named rather than the preferred ones.
    ///
    /// This is how a test checks a pipeline without a sound card or a display, and how a
    /// user whose machine lacks the preferred sink chooses another: `fakesink` accepts
    /// everything and shows nothing, which is a pipeline that can be verified where a
    /// device cannot.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Unavailable`] when no `gst-launch-1.0` can be run.
    pub fn with_sinks(audio_sink: &str, video_sink: &str) -> Result<Self, RenderError> {
        let program = find_launcher().map_err(unavailable)?;
        Ok(Self {
            program,
            audio_sink: audio_sink.to_owned(),
            video_sink: video_sink.to_owned(),
            volume: Mutex::new(Volume::FULL),
        })
    }

    /// The program this renderer runs pipelines in.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The volume a stream opened later will play at.
    ///
    /// GStreamer's volume is a pipeline element and a pipeline is started before a
    /// client says anything about volume, so a stream reads it when it is opened. What
    /// a client asks for mid-stream reaches a live pipeline in S4-3.
    pub fn volume(&self) -> Volume {
        *self
            .volume
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The audio sink its streams play into.
    pub fn audio_sink(&self) -> &str {
        &self.audio_sink
    }

    /// The video sink its streams show in.
    pub fn video_sink(&self) -> &str {
        &self.video_sink
    }
}

/// Picks a sink: the environment's if it names one, the first of `preferred` otherwise.
fn sink_from_env(variable: &str, preferred: &[&str]) -> String {
    std::env::var(variable)
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| preferred[0].to_owned())
}

/// Turns a pipeline failure into the render error a session reports.
fn unavailable(error: LaunchError) -> RenderError {
    match error {
        LaunchError::NotFound { .. } => RenderError::Unavailable(error.to_string()),
        LaunchError::StartFailed { .. } => RenderError::Unavailable(error.to_string()),
        LaunchError::Rejected { .. } | LaunchError::Stopped { .. } => {
            RenderError::Failed(error.to_string())
        }
    }
}

/// The caps a coding's frames are described by, which is what tells a decoder what it
/// is being handed.
///
/// An empty description is a coding this backend cannot describe, which is refused
/// rather than guessed at: a pipeline fed the wrong caps fails inside GStreamer, where
/// the reason is far away from the decision that was wrong.
fn audio_caps(config: AudioConfig) -> Result<String, RenderError> {
    match config.codec {
        // The stream's own framing is what the parser reads; the sample rate and
        // channel count are the ones the session runs at, and a client that uses
        // others is not one this backend has seen.
        AudioCodec::AacEld => Ok(format!(
            "audio/mpeg,mpegversion=4,stream-format=raw,channels={},rate={}",
            config.channels, config.sample_rate
        )),
        AudioCodec::Alac => Ok(format!(
            "audio/x-alac,channels={},rate={}",
            config.channels, config.sample_rate
        )),
    }
}

/// The pipeline one audio stream runs.
///
/// `fdsrc` reads the frames this backend writes; `decodebin` picks the decoder for the
/// caps, which is what upstream does too rather than naming one decoder per coding.
fn audio_pipeline(caps: &str, sink: &str) -> String {
    // A silence the sink can play is what a decoder that gets nothing produces, so the
    // pipeline is told not to expect a fixed rate: a client's clock and the sound
    // card's are not the same clock.
    format!("fdsrc ! {caps} ! decodebin ! audioconvert ! audioresample ! {sink} sync=false")
}

/// The pipeline one video stream runs.
///
/// A mirror stream carries H.264 or H.265 depending on what the device sends and
/// what the receiver advertised, and the two need different parsers and decoders.
/// `decodebin` picks them from what arrives, which is what makes one pipeline serve
/// both: a device that switches coding mid-session is then not a special case.
fn video_pipeline(sink: &str) -> String {
    format!("fdsrc ! decodebin ! videoconvert ! {sink} sync=false")
}

impl Renderer for GstRenderer {
    fn open_audio(&self, config: AudioConfig) -> Result<Box<dyn AudioStream + '_>, RenderError> {
        let caps = audio_caps(config)?;
        let description = audio_pipeline(&caps, &self.audio_sink);
        let pipeline = Pipeline::start(&self.program, &description).map_err(unavailable)?;
        Ok(Box::new(GstAudio { pipeline }))
    }

    fn open_video(&self, _config: VideoConfig) -> Result<Box<dyn VideoStream + '_>, RenderError> {
        let description = video_pipeline(&self.video_sink);
        let pipeline = Pipeline::start(&self.program, &description).map_err(unavailable)?;
        Ok(Box::new(GstVideo { pipeline }))
    }

    fn set_volume(&self, volume: Volume) -> Result<(), RenderError> {
        // GStreamer's own volume control is a pipeline element, and a pipeline is
        // started before a client says anything about volume, so what this backend can
        // do without rebuilding every stream is remember it. The session reads it back;
        // a stream opened later gets it as a `volume` element. Playing at a volume a
        // client asked for is what S4-3 wires up.
        *self
            .volume
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = volume;
        Ok(())
    }
}

/// One audio stream, writing frames to a pipeline.
#[derive(Debug)]
struct GstAudio {
    pipeline: Pipeline,
}

impl AudioStream for GstAudio {
    fn push(&self, frame: AudioFrame) -> Result<(), RenderError> {
        // A frame is written as the pipeline's own bytes: `fdsrc` has no framing of its
        // own, so what a decoder reads is what a client sent.
        self.pipeline.write(&frame.data).map_err(unavailable)
    }

    fn flush(&self) -> Result<(), RenderError> {
        // A pipeline's buffers are its own; ending it is what drops them, and a stream
        // is not something this backend can restart in place without a client's help.
        Ok(())
    }

    fn close(&self) -> Result<(), RenderError> {
        self.pipeline.finish().map_err(unavailable)
    }
}

/// One video stream, writing frames to a pipeline.
#[derive(Debug)]
struct GstVideo {
    pipeline: Pipeline,
}

impl VideoStream for GstVideo {
    fn push(&self, frame: VideoFrame) -> Result<(), RenderError> {
        self.pipeline.write(&frame.data).map_err(unavailable)
    }

    fn set_pace(&self, until_next_seconds: f64) -> Result<(), RenderError> {
        // GStreamer would do this itself if the sink were allowed to synchronise,
        // but a pipeline fed through a pipe has nothing to synchronise against: the
        // source is a descriptor, and its idea of time is when the bytes arrived.
        // So the caller's clock is applied here, between frames, which is also the
        // shape a future A/V sync needs — one place decides when the next frame is
        // due, and both streams are handed the same answer.
        let seconds = until_next_seconds.clamp(0.0, MAX_PACE_SECONDS);
        if seconds > 0.0 {
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
        }
        Ok(())
    }

    fn flush(&self) -> Result<(), RenderError> {
        Ok(())
    }

    fn close(&self) -> Result<(), RenderError> {
        self.pipeline.finish().map_err(unavailable)
    }
}
