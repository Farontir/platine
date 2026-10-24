//! Keeps the viewport's environment in step with the document.
//!
//! The GPU is a cache of the document: when the environment source of the document (or
//! of the draft, without a project) differs from the one on the GPU, the image is decoded
//! on a worker thread and handed to the viewport. Undoing a change of HDRI therefore
//! needs no special code.

use std::collections::BTreeMap;
use std::sync::mpsc::{Receiver, TryRecvError};

use patine_core::environment::EnvironmentSource;
use patine_core::resource::{Resource, ResourceHash};
use patine_render::environment::{EquirectImage, studio_sky};

use crate::display_panel::{STUDIO_SKY_WIDTH, environments_dir};

type Decoded = (EnvironmentSource, Result<EquirectImage, String>);

/// Decoding state of the viewport's environment.
pub struct EnvironmentSync {
    /// Source on the GPU, or being decoded for it.
    requested: EnvironmentSource,
    task: Option<Receiver<Decoded>>,
    /// Message of the last failure.
    pub error: Option<String>,
}

impl Default for EnvironmentSync {
    fn default() -> Self {
        // The viewport starts with the studio sky.
        Self {
            requested: EnvironmentSource::Studio,
            task: None,
            error: None,
        }
    }
}

impl std::fmt::Debug for EnvironmentSync {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentSync")
            .field("requested", &self.requested)
            .field("loading", &self.task.is_some())
            .field("error", &self.error)
            .finish()
    }
}

/// The decoding of `source`, to run on a worker thread.
pub(crate) fn decode(
    source: &EnvironmentSource,
    resources: &BTreeMap<ResourceHash, Resource>,
) -> Box<dyn FnOnce() -> Result<EquirectImage, String> + Send> {
    match source {
        EnvironmentSource::Studio => Box::new(|| Ok(studio_sky(STUDIO_SKY_WIDTH))),
        EnvironmentSource::Bundled(name) => {
            let path = environments_dir().map(|dir| dir.join(name));
            let name = name.clone();
            Box::new(move || {
                let path = path.ok_or_else(|| format!("{name}: assets/environments not found"))?;
                EquirectImage::load(&path).map_err(|error| error.to_string())
            })
        }
        EnvironmentSource::Resource(hash) => match resources.get(hash) {
            Some(resource) => {
                let bytes = resource.bytes.clone();
                Box::new(move || EquirectImage::decode(&bytes).map_err(|error| error.to_string()))
            }
            None => {
                let hash = *hash;
                Box::new(move || Err(format!("missing resource {hash}")))
            }
        },
    }
}

impl EnvironmentSync {
    /// Whether an image is being decoded.
    pub fn loading(&self) -> bool {
        self.task.is_some()
    }

    /// Records that the viewport already shows `source` (processed under the loading
    /// page), or failed to with `error`.
    pub fn shown(&mut self, source: EnvironmentSource, error: Option<String>) {
        self.requested = source;
        self.task = None;
        self.error = error;
    }

    /// Records that `image`, already decoded, shows `source` (an HDRI the user just
    /// picked): it is returned by the next [`EnvironmentSync::update`] without decoding.
    pub fn preload(
        &mut self,
        ctx: &egui::Context,
        source: EnvironmentSource,
        image: EquirectImage,
    ) {
        let (sender, receiver) = std::sync::mpsc::channel();
        // The receiver is alive: the send cannot fail.
        let _ = sender.send((source.clone(), Ok(image)));
        self.requested = source;
        self.task = Some(receiver);
        self.error = None;
        ctx.request_repaint();
    }

    /// Starts decoding `wanted` if the GPU shows something else, and returns a decoded
    /// image to process when one is ready.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        wanted: &EnvironmentSource,
        resources: &BTreeMap<ResourceHash, Resource>,
    ) -> Option<EquirectImage> {
        if *wanted != self.requested {
            self.requested = wanted.clone();
            self.error = None;
            let job = decode(wanted, resources);
            let source = wanted.clone();
            let (sender, receiver) = std::sync::mpsc::channel();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                tracing::info!(?source, "decoding environment");
                // The request may have been superseded; nobody listens then.
                let _ = sender.send((source, job()));
                ctx.request_repaint();
            });
            // A previous decode, if any, is abandoned.
            self.task = Some(receiver);
        }
        let task = self.task.as_ref()?;
        match task.try_recv() {
            Ok((source, result)) => {
                self.task = None;
                if source != self.requested {
                    return None;
                }
                match result {
                    Ok(image) => Some(image),
                    Err(message) => {
                        tracing::error!(%message, "environment decoding failed");
                        self.error = Some(message);
                        None
                    }
                }
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.task = None;
                None
            }
        }
    }
}
