//! Who owns the renderer (BL144.10): one policy for exports and library thumbnails.
//!
//! A renderer is a browser and the GPU. Two at once fight for both, so the host lets one job at a
//! time own it. This module is the rule; the adapter that runs the jobs (`tessel-editor-media`'s
//! `MediaHost`) only reads the registry, asks [`decide`] and carries out what it says. Nothing here
//! knows a window, a socket, a clock or a renderer.
//!
//! ```text
//!                         asks for the renderer
//!   holds it              export / preflight        thumbnails (window tick or explicit)
//!   ------------------    ----------------------    ------------------------------------
//!   nobody                granted                   granted
//!   an export/preflight   refused, retryable        refused, retryable
//!   a thumbnail job       refused, retryable, and   its own family decides (same project:
//!                         the job is asked to stop  reuse; other project: wait), never a
//!                         (export has priority)     second renderer
//!   reserved for export   (granted: it is the one   refused, retryable
//!                         that reserved it)
//! ```
//!
//! **Nothing is queued.** A refusal is an answer, not a promise: the request is not remembered and
//! the caller must ask again. What a refusal *can* do is make the retry succeed: when an export
//! (or its preflight) is refused only because a thumbnail job holds the renderer, that job is asked
//! to stop at its next frame boundary and the renderer is **reserved** for the export for a bounded
//! time, so the window's tick does not hand it to a new thumbnail job before the caller retries
//! (otherwise the export would starve behind a tick that always wins). The reservation ends when an
//! export is granted or when it expires; it never outlives a project change in a way that loses
//! anything, because it only delays work nobody asked for.
//!
//! The user's decisions are not this module's to undo: a thumbnail job the user cancelled leaves
//! that project's automatic drawing paused whatever the exports do, and a thumbnail job stopped on
//! behalf of an export is *not* the user's cancel (the host tells them apart).

use serde_json::json;

use crate::jobs::{JobId, JobInfo, JobKind};
use crate::state::CommandError;

/// How long a refused export keeps the renderer free of new thumbnail work.
pub const EXPORT_PRIORITY_SECONDS: u64 = 30;

/// Whether every job of this kind builds a renderer, so that it counts while it is not over.
///
/// `Preflight` is not here on purpose: the kind also covers checks that never touch a renderer
/// (package preflight, loading scene resources). The adapter registers the one preflight that
/// probes the renderer's limits as a [`Holder`] of kind `Preflight` itself.
pub const fn uses_renderer(kind: JobKind) -> bool {
    matches!(kind, JobKind::Thumbnail | JobKind::VideoExport)
}

/// A job that owns the renderer: it is registered and not over (a cancelled job still owns it
/// until its body returns, because a frame in flight cannot be interrupted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub job: JobId,
    pub kind: JobKind,
}

impl Holder {
    /// The holder `info` describes, or `None` when it is over or its kind does not always use a
    /// renderer (see [`uses_renderer`]).
    pub fn of(info: &JobInfo) -> Option<Self> {
        (!info.state.is_final() && uses_renderer(info.kind)).then(|| Self {
            job: info.id.clone(),
            kind: info.kind,
        })
    }

    fn is_thumbnail(&self) -> bool {
        self.kind == JobKind::Thumbnail
    }
}

/// Whoever asks for the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claimant {
    /// Thumbnails nobody asked for: the window's tick.
    AutomaticThumbnails,
    /// `media.thumbnails` from a caller.
    ExplicitThumbnails,
    /// `export.preflight` / `scene.export.preflight` job that probes the renderer's limits.
    Preflight,
    /// `export.start` or `scene.export`.
    Export,
}

impl Claimant {
    /// Exports and their preflight have priority over thumbnails.
    pub fn is_export_class(self) -> bool {
        matches!(self, Self::Preflight | Self::Export)
    }
}

/// What the host must do instead of starting the job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    /// `renderer_busy` when an export owns (or is reserved) the renderer or a thumbnail job holds it
    /// against an export; `thumbnail_busy` when another thumbnail job holds it against thumbnails.
    pub code: &'static str,
    pub message: String,
    /// The job in the way, if one is.
    pub holder: Option<Holder>,
    /// A thumbnail job the host must ask to stop (export priority). Never an export.
    pub preempt: Option<JobId>,
    /// The host must reserve the renderer for an export for [`EXPORT_PRIORITY_SECONDS`].
    pub reserve: bool,
    /// Thumbnails against a thumbnail job: the thumbnail family arbitrates (reuse, or wait for the
    /// job of another project). Only the host knows its stamps.
    pub same_family: bool,
    /// The refusal is only the renderer being reserved for an export.
    pub reserved: bool,
}

impl Refusal {
    /// The reply of a refused request: always retryable, never a promise to retry for the caller.
    pub fn error(&self) -> CommandError {
        let mut details = json!({
            "retryable": true,
            "queued": false,
            "reserved_for_export": self.reserved || self.reserve,
        });
        if let Some(holder) = &self.holder {
            details["job"] = json!(holder.job);
            details["job_kind"] = json!(holder.kind.label());
        }
        if self.preempt.is_some() {
            details["stopping"] = json!(true);
        }
        CommandError::new(self.code, self.message.clone()).details(details)
    }
}

/// The answer to one request for the renderer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Grant,
    Refuse(Refusal),
}

/// Decides one request. `holders` are the jobs that own the renderer now (see [`Holder::of`]);
/// `reserved` says the renderer is reserved for an export that was refused behind a thumbnail job.
///
/// Pure and total: the same registry and reservation always give the same answer, and the order of
/// `holders` only decides which of several equal holders is named.
pub fn decide(claimant: Claimant, holders: &[Holder], reserved: bool) -> Verdict {
    let export_holder = holders.iter().find(|holder| !holder.is_thumbnail());
    let thumbnail_holder = holders.iter().find(|holder| holder.is_thumbnail());
    let refusal = |code, message: String| Refusal {
        code,
        message,
        holder: None,
        preempt: None,
        reserve: false,
        same_family: false,
        reserved: false,
    };
    if claimant.is_export_class() {
        if let Some(holder) = export_holder {
            return Verdict::Refuse(Refusal {
                holder: Some(holder.clone()),
                ..refusal(
                    "renderer_busy",
                    format!(
                        "The renderer belongs to the {} job `{}`; retry when it has finished",
                        holder.kind.label(),
                        holder.job
                    ),
                )
            });
        }
        if let Some(holder) = thumbnail_holder {
            return Verdict::Refuse(Refusal {
                holder: Some(holder.clone()),
                preempt: Some(holder.job.clone()),
                reserve: true,
                ..refusal(
                    "renderer_busy",
                    format!(
                        "A thumbnail job (`{}`) holds the renderer; it was asked to stop at its next \
                         frame and the renderer is reserved for the export: ask again in a moment",
                        holder.job
                    ),
                )
            });
        }
        return Verdict::Grant;
    }
    if let Some(holder) = export_holder {
        return Verdict::Refuse(Refusal {
            holder: Some(holder.clone()),
            ..refusal(
                "renderer_busy",
                format!(
                    "The {} job `{}` owns the renderer; retry thumbnails after it has finished",
                    holder.kind.label(),
                    holder.job
                ),
            )
        });
    }
    if reserved {
        return Verdict::Refuse(Refusal {
            holder: thumbnail_holder.cloned(),
            reserved: true,
            ..refusal(
                "renderer_busy",
                "The renderer is reserved for an export that was just refused; retry thumbnails \
                 after it has run (or in a moment)"
                    .to_string(),
            )
        });
    }
    if let Some(holder) = thumbnail_holder {
        return Verdict::Refuse(Refusal {
            holder: Some(holder.clone()),
            same_family: true,
            ..refusal(
                "thumbnail_busy",
                format!(
                    "The thumbnail job `{}` is still ending or drawing; retry this request after it finishes",
                    holder.job
                ),
            )
        });
    }
    Verdict::Grant
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holder(job: &str, kind: JobKind) -> Holder {
        Holder {
            job: job.to_string(),
            kind,
        }
    }

    fn refusal(verdict: Verdict) -> Refusal {
        match verdict {
            Verdict::Refuse(refusal) => refusal,
            Verdict::Grant => panic!("expected a refusal"),
        }
    }

    #[test]
    fn an_idle_renderer_is_granted_to_everyone() {
        for claimant in [
            Claimant::AutomaticThumbnails,
            Claimant::ExplicitThumbnails,
            Claimant::Preflight,
            Claimant::Export,
        ] {
            assert_eq!(decide(claimant, &[], false), Verdict::Grant, "{claimant:?}");
        }
    }

    #[test]
    fn only_jobs_that_build_a_renderer_count() {
        for kind in [
            JobKind::Import,
            JobKind::Hash,
            JobKind::Probe,
            // Package preflight and scene-resource loading share this kind and use no renderer:
            // the adapter registers the renderer probe itself.
            JobKind::Preflight,
            JobKind::Proxy,
            JobKind::Package,
            JobKind::Restore,
            JobKind::ProjectImport,
            JobKind::Discard,
        ] {
            assert!(!uses_renderer(kind), "{kind:?}");
        }
        for kind in [JobKind::Thumbnail, JobKind::VideoExport] {
            assert!(uses_renderer(kind), "{kind:?}");
        }
    }

    #[test]
    fn an_export_in_flight_refuses_every_other_request_without_stopping_it() {
        let holders = [holder("job-1", JobKind::VideoExport)];
        for claimant in [
            Claimant::AutomaticThumbnails,
            Claimant::ExplicitThumbnails,
            Claimant::Preflight,
            Claimant::Export,
        ] {
            let refused = refusal(decide(claimant, &holders, false));
            assert_eq!(refused.code, "renderer_busy", "{claimant:?}");
            assert_eq!(refused.holder, Some(holders[0].clone()));
            assert_eq!(refused.preempt, None, "an export is never preempted");
            assert!(!refused.reserve, "{claimant:?}");
            let error = refused.error();
            assert_eq!(error.code, "renderer_busy");
            let details = error.details.as_deref().expect("details");
            assert_eq!(details["retryable"], true);
            assert_eq!(details["queued"], false, "nothing is queued");
        }
    }

    #[test]
    fn a_thumbnail_job_yields_to_an_export_and_the_renderer_is_reserved() {
        let holders = [holder("job-7", JobKind::Thumbnail)];
        for claimant in [Claimant::Export, Claimant::Preflight] {
            let refused = refusal(decide(claimant, &holders, false));
            assert_eq!(refused.code, "renderer_busy");
            assert_eq!(refused.preempt.as_deref(), Some("job-7"));
            assert!(refused.reserve, "{claimant:?}");
            assert!(!refused.same_family);
            let error = refused.error();
            let details = error.details.as_deref().expect("details");
            assert_eq!(details["retryable"], true);
            assert_eq!(details["stopping"], true);
            assert_eq!(details["job"], "job-7");
        }
    }

    #[test]
    fn thumbnails_never_take_the_renderer_from_a_thumbnail_job_or_an_export_reservation() {
        let holders = [holder("job-7", JobKind::Thumbnail)];
        for claimant in [Claimant::AutomaticThumbnails, Claimant::ExplicitThumbnails] {
            let refused = refusal(decide(claimant, &holders, false));
            assert_eq!(refused.code, "thumbnail_busy");
            assert!(refused.same_family, "the host arbitrates its own family");
            assert_eq!(refused.preempt, None);
            assert!(!refused.reserve);

            let reserved = refusal(decide(claimant, &holders, true));
            assert_eq!(reserved.code, "renderer_busy", "the reservation wins");
            assert!(reserved.reserved && !reserved.same_family);

            let reserved = refusal(decide(claimant, &[], true));
            assert_eq!(reserved.code, "renderer_busy");
            assert_eq!(reserved.holder, None);
        }
    }

    #[test]
    fn an_export_is_granted_while_its_own_reservation_stands() {
        assert_eq!(decide(Claimant::Export, &[], true), Verdict::Grant);
        assert_eq!(decide(Claimant::Preflight, &[], true), Verdict::Grant);
    }

    #[test]
    fn an_export_and_a_thumbnail_job_in_the_registry_name_the_export_first() {
        // Never expected (the policy keeps it from happening) but the answer must still be safe.
        let holders = [
            holder("job-1", JobKind::Thumbnail),
            holder("job-2", JobKind::VideoExport),
        ];
        let refused = refusal(decide(Claimant::Export, &holders, false));
        assert_eq!(refused.holder, Some(holders[1].clone()));
        assert_eq!(refused.preempt, None);
        let refused = refusal(decide(Claimant::ExplicitThumbnails, &holders, false));
        assert_eq!(refused.holder, Some(holders[1].clone()));
        assert!(!refused.same_family);
    }
}
