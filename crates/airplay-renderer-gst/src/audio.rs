// SPDX-License-Identifier: GPL-3.0-or-later
//! The audio stream: an `appsrc` the session pushes frames into.
//!
//! Audio cannot be delivered the way video is. A socket is a stream of bytes either way,
//! but H.264 has start codes, so a decoder can find its frames in a run of them, and raw
//! AAC-ELD has no such thing: a frame carries no sample rate of its own and there is no
//! sync word to split on. The only way a decoder can be given these frames is one buffer
//! per frame, which is what upstream does and why upstream plays sound
//! (`upstream/UxPlay@3dbf7cee:renderers/audio_renderer.c:332-367`).
//!
//! `appsrc` is that buffer, and it is why audio is linked into this process while video is
//! still run as a `gst-launch` subprocess.
//!
//! Two details are the whole of what makes it work, and both were measured:
//!
//! - **the caps carry `codec_data`**, the MPEG-4 `AudioSpecificConfig` that says what the
//!   frames are. A raw frame cannot say it, so a decoder without it cannot fix its output
//!   format and refuses the stream with "output caps are unfixed". The values are
//!   upstream's (`audio_renderer.c:60-68`), and the one this port had guessed for ELD was
//!   not even ELD — its object type decodes to 33 rather than 39;
//! - **one push is one frame**, because that is the only boundary a raw AAC stream has.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use airplay_renderer::{AudioCodec, AudioConfig, AudioFrame, AudioStream, RenderError};
use gstreamer as gst;
use gstreamer::glib::prelude::*;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;

/// Where a pipeline's failure is kept, so a push can say why nothing is being played.
type Failure = Arc<Mutex<Option<String>>>;

/// The caps for a coding, including the configuration its frames do not carry.
///
/// The `codec_data` is the part that cannot be derived from the frames: it is the
/// `AudioSpecificConfig` (for AAC) or the magic cookie (for ALAC) that a decoder needs
/// before it can read one. Both values are upstream's own
/// (`upstream/UxPlay@3dbf7cee:renderers/audio_renderer.c:60-68`).
fn audio_caps(config: AudioConfig) -> Result<gst::Caps, RenderError> {
    let text = match config.codec {
        AudioCodec::AacEld => format!(
            "audio/mpeg,mpegversion=(int)4,channels=(int){},rate=(int){},stream-format=raw,\
             codec_data=(buffer)f8e85000",
            config.channels, config.sample_rate
        ),
        AudioCodec::Alac => format!(
            "audio/x-alac,mpegversion=(int)4,channels=(int){},rate=(int){},stream-format=raw,\
             codec_data=(buffer)00000024616c616300000000000001600010280a0e0200ff0000000000000000\
             0000ac44",
            config.channels, config.sample_rate
        ),
    };
    text.parse::<gst::Caps>()
        .map_err(|_| RenderError::Unavailable(format!("GStreamer did not accept the caps {text}")))
}

/// The decoder a coding needs, named rather than searched for.
///
/// `decodebin` is right for video, which changes coding mid-session, and wrong here: the
/// frame this stream carries has no parser to introduce it, so the decoder has to be
/// chosen from the coding the client named.
fn audio_decoder(codec: AudioCodec) -> &'static str {
    match codec {
        AudioCodec::AacEld => "avdec_aac",
        AudioCodec::Alac => "avdec_alac",
    }
}

/// The description one audio pipeline is built from.
///
/// `queue` between the source and the decoder is what lets the two run on different
/// threads; `audioconvert` and `audioresample` are there because a sink rarely takes
/// 44.1 kHz stereo exactly, and a resampler told nothing is a resampler that picks a poor
/// quality. `sync=false` because a client's clock and a sound card's are not the same
/// clock, and waiting on one to schedule the other is what makes a receiver stutter.
fn audio_description(config: AudioConfig, sink: &str) -> String {
    format!(
        "appsrc name=audio_source ! queue ! {} ! audioconvert ! audioresample quality=10 ! {sink} \
         sync=false",
        audio_decoder(config.codec)
    )
}

/// One audio stream: an `appsrc`, and the pipeline whose thread outlives it.
///
/// The pipeline runs a GLib main loop on a thread of its own, because a bus nobody reads
/// is a failure nobody hears about, and `appsrc` is only ever as good as the pipeline it
/// feeds. Stopping that thread is [`Drop`]'s job: a stream that ended without it would
/// leave a sound card open.
pub(crate) struct GstAudio {
    appsrc: AppSrc,
    /// What the pipeline said was wrong, if it said anything.
    failure: Failure,
    /// Stops the thread that runs the pipeline's main loop, when dropped.
    stop: Option<Sender<()>>,
    /// How many frames were pushed, so a silent session can be told from a stalled one.
    pushed: AtomicU64,
    /// The rate a frame's timestamp is counted in, which is what makes it a time.
    ///
    /// `AudioFrame::timestamp` is the client's own clock, and for audio that clock counts
    /// samples: a session at 44.1 kHz stamps its frames 480 apart. Treating that number as
    /// nanoseconds put every buffer at a time that had nothing to do with when it plays,
    /// which `sync=false` hid and would not have hidden for long.
    sample_rate: u32,
}

impl std::fmt::Debug for GstAudio {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GstAudio")
            .field("pushed", &self.pushed.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl GstAudio {
    /// Builds the pipeline and starts the thread that runs it.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError::Unavailable`] when an element is missing or the caps cannot
    /// be described, and [`RenderError::Failed`] when the pipeline will not start.
    pub(crate) fn open(config: AudioConfig, sink: &str) -> Result<Self, RenderError> {
        gst::init().map_err(|error| {
            RenderError::Unavailable(format!("GStreamer cannot be initialised: {error}"))
        })?;
        let caps = audio_caps(config)?;
        let description = audio_description(config, sink);
        let failure: Failure = Arc::new(Mutex::new(None));
        let watched = Arc::clone(&failure);
        let (ready, started) = std::sync::mpsc::channel::<Result<AppSrc, String>>();
        let (stop, stopping) = std::sync::mpsc::channel::<()>();

        std::thread::Builder::new()
            .name("airplay-gst-audio".to_owned())
            .spawn(move || run(&description, caps, watched, ready, stopping))
            .map_err(|error| {
                RenderError::Unavailable(format!("the audio thread cannot start: {error}"))
            })?;

        let appsrc = started
            .recv()
            .map_err(|_| RenderError::Failed("the audio pipeline died at startup".to_owned()))?
            .map_err(RenderError::Unavailable)?;
        Ok(Self {
            appsrc,
            failure,
            stop: Some(stop),
            pushed: AtomicU64::new(0),
            sample_rate: config.sample_rate.max(1),
        })
    }
}

/// Runs one pipeline until it is told to stop, reporting its first failure.
///
/// Everything here happens inside the pipeline's own main context: GStreamer gives an
/// element the thread-default context of whoever creates it, and a bus watch added to a
/// context nobody runs never reports anything. The context is entered with
/// `with_thread_default` and the loop runs inside it, which is the whole of the
/// arrangement.
fn run(
    description: &str,
    caps: gst::Caps,
    failure: Failure,
    ready: Sender<Result<AppSrc, String>>,
    stopping: Receiver<()>,
) {
    let context = gst::glib::MainContext::new();
    let loop_ = gst::glib::MainLoop::new(Some(&context), false);
    let telling = ready.clone();
    let entered = context.with_thread_default(|| {
        let element = match gst::parse::launch(description) {
            Ok(element) => element,
            Err(error) => {
                let _ = ready.send(Err(format!("the audio pipeline cannot be built: {error}")));
                return;
            }
        };
        let pipeline = match element.downcast::<gst::Pipeline>() {
            Ok(pipeline) => pipeline,
            Err(_) => {
                let _ = ready.send(Err("the audio pipeline is not a pipeline".to_owned()));
                return;
            }
        };
        let source = pipeline
            .by_name("audio_source")
            .and_then(|element| element.downcast::<AppSrc>().ok());
        let Some(appsrc) = source else {
            let _ = ready.send(Err("the audio pipeline has no source".to_owned()));
            return;
        };

        // What the pipeline is handed, said once and for all: the caps go on the source
        // rather than on a filter downstream, because it is the source that has to say what
        // its own buffers are.
        appsrc.set_caps(Some(&caps));
        appsrc.set_format(gst::Format::Time);

        // The guard is held for as long as the loop runs: dropping it removes the watch.
        let _watching = {
            let quiet = Arc::clone(&failure);
            let loop_for_bus = loop_.clone();
            let bus = pipeline.bus().expect("a pipeline has a bus");
            bus.add_watch_local(move |_, message| {
                use gst::MessageView;
                match message.view() {
                    MessageView::Error(error) => {
                        let text = format!(
                            "{}: {}",
                            error
                                .src()
                                .map(|source| source.name().to_string())
                                .unwrap_or_else(|| "the audio pipeline".to_owned()),
                            error.error()
                        );
                        let mut kept = quiet
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if kept.is_none() {
                            *kept = Some(text);
                        }
                        loop_for_bus.quit();
                        gst::glib::ControlFlow::Break
                    }
                    MessageView::Eos(_) => {
                        loop_for_bus.quit();
                        gst::glib::ControlFlow::Break
                    }
                    _ => gst::glib::ControlFlow::Continue,
                }
            })
            .expect("the bus watch is added")
        };

        if let Err(error) = pipeline.set_state(gst::State::Playing) {
            let _ = ready.send(Err(format!("the audio pipeline will not start: {error}")));
            return;
        }
        // Handed out only once the pipeline is playing, so no frame can be pushed into a
        // pipeline that is not ready for it: `appsrc` would take them and the state change
        // would drop them, which is a moment of silence at the start of every session.
        let _ = ready.send(Ok(appsrc));

        // The thread ends when the pipeline fails, ends, or is told to stop. `quit` is safe
        // to call from another thread and wakes the loop, so a small thread waits on the
        // channel and quits — which is also what happens when the sender is dropped, so a
        // stream that went away without closing still ends its pipeline.
        {
            let loop_for_stop = loop_.clone();
            std::thread::Builder::new()
                .name("airplay-gst-audio-stop".to_owned())
                .spawn(move || {
                    let _ = stopping.recv();
                    loop_for_stop.quit();
                })
                .expect("the stop thread starts");
        }

        loop_.run();
        let _ = pipeline.set_state(gst::State::Null);
    });
    if entered.is_err() {
        let _ = telling.send(Err(
            "the audio pipeline's main context is owned by another thread".to_owned(),
        ));
    }
}
impl AudioStream for GstAudio {
    fn push(&self, frame: AudioFrame) -> Result<(), RenderError> {
        if let Some(reason) = self
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
        {
            return Err(RenderError::Unavailable(reason));
        }
        // One buffer, one frame. This is the whole of the delivery rule, and the reason
        // this stream is not a subprocess: a byte stream cannot say where a frame ends,
        // and a decoder given a run of frames reads only the first one.
        let mut buffer = gst::Buffer::from_slice(frame.data);
        if let Some(buffer) = buffer.get_mut() {
            // Samples to nanoseconds, which is the only clock a buffer has. The absolute
            // value is the client's rather than this machine's: turning it into a play
            // time needs a base taken from the first frame and an offset from the clock
            // exchange, which is what synchronising playback will do — until then the sink
            // is told not to wait, so nothing here is scheduled against it.
            let seconds = frame.timestamp / u64::from(self.sample_rate);
            let nanos = frame.timestamp % u64::from(self.sample_rate) * 1_000_000_000
                / u64::from(self.sample_rate);
            buffer.set_pts(gst::ClockTime::from_nseconds(
                seconds.saturating_mul(1_000_000_000).saturating_add(nanos),
            ));
        }
        self.appsrc.push_buffer(buffer).map_err(|error| {
            RenderError::Failed(format!("the pipeline refused a frame: {error}"))
        })?;
        self.pushed.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn flush(&self) -> Result<(), RenderError> {
        // A pipeline's buffers are its own; ending it is what drops them, and a stream is
        // not something this backend can restart in place without a client's help.
        Ok(())
    }

    fn close(&self) -> Result<(), RenderError> {
        // The frames already handed over are played out rather than cut off, which is what
        // an end of stream means to a pipeline: the decoder drains, then the branch ends.
        self.appsrc
            .end_of_stream()
            .map_err(|error| RenderError::Failed(format!("the stream would not end: {error}")))?;
        Ok(())
    }
}

impl Drop for GstAudio {
    fn drop(&mut self) {
        // The thread owns the pipeline and the sound card it opened, so it is told to stop
        // and waited for: a stream that went away while its pipeline was still playing is
        // a sound card held open by a session that ended.
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}
