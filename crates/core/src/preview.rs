//! Preview quality policy: when the monitor may draw a smaller derived copy of a video instead
//! of the original, and when it must not.
//!
//! The rule lives here, once, for the monitor, the CLI and the media adapter:
//!
//! * `Original` never uses nor builds a proxy.
//! * `Auto` builds a proxy for a video taller than `max_height` and uses it **only for
//!   interactive frames**. Everything that asks for pixels (explicit `render.frame`, probes, PNG
//!   output, export) draws the original, and the monitor refines the picture to the original
//!   when the scrub settles (`ViewportSession::dispatch_pending`).
//!
//! Quality is never hidden: every frame answer says how many layers used a proxy and the
//! policy can be queried with the reason for each asset (`preview.policy`).

use serde_json::{Value, json};

use crate::assets::{Asset, MediaKind, ProxyFormat, ProxyFrames};
use crate::state::EditorState;

/// Proxy heights offered by default; 540 keeps 1080p sources at an exact half scale.
pub const DEFAULT_MAX_HEIGHT: u32 = 540;
/// Below this a proxy stops being a preview of the picture.
pub const MIN_HEIGHT: u32 = 144;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewMode {
    Original,
    #[default]
    Auto,
}

impl PreviewMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "original",
            Self::Auto => "auto",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "original" => Some(Self::Original),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

/// Quality a frame is drawn at. `Preview` allows the runtime to use an attached proxy per
/// layer; a layer without a (valid) proxy is drawn from the original either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    Preview,
    #[default]
    Original,
}

impl Quality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preview => "preview",
            Self::Original => "original",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewPolicy {
    pub mode: PreviewMode,
    /// Height of the derived proxy, and the height above which a video gets one.
    pub max_height: u32,
    pub format: ProxyFormat,
}

impl Default for PreviewPolicy {
    fn default() -> Self {
        Self {
            mode: PreviewMode::Auto,
            max_height: DEFAULT_MAX_HEIGHT,
            format: ProxyFormat::Jpeg,
        }
    }
}

/// Why an asset is (not) drawn from a proxy, as the policy sees it right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    NotVideo,
    PolicyOriginal,
    NoFrames,
    SmallEnough,
    Missing,
    Stale,
    Ready,
}

impl Reason {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotVideo => "not_video",
            Self::PolicyOriginal => "policy_original",
            Self::NoFrames => "no_frames",
            Self::SmallEnough => "small_enough",
            Self::Missing => "proxy_missing",
            Self::Stale => "proxy_stale",
            Self::Ready => "proxy_ready",
        }
    }
}

impl PreviewPolicy {
    pub fn to_json(self) -> Value {
        json!({ "mode": self.mode.label(), "max_height": self.max_height,
            "format": self.format.label() })
    }

    /// Quality of an interactive frame (the monitor's scrub); explicit frames are always
    /// `Original`, they never ask this.
    pub fn interactive_quality(self) -> Quality {
        match self.mode {
            PreviewMode::Original => Quality::Original,
            PreviewMode::Auto => Quality::Preview,
        }
    }

    /// Whether a proxy should exist for this asset, and why not. `Ready`/`Missing`/`Stale`
    /// say what the asset has now; only `Missing` and `Stale` mean "build one".
    pub fn assess(self, kind: MediaKind, asset: Option<&Asset>) -> Reason {
        if kind != MediaKind::Video {
            return Reason::NotVideo;
        }
        if self.mode == PreviewMode::Original {
            return Reason::PolicyOriginal;
        }
        let Some(decoded) = asset.and_then(|asset| asset.decoded.as_ref()) else {
            return Reason::NoFrames;
        };
        let Some(frames) = &decoded.frames else {
            return Reason::NoFrames;
        };
        let height = asset.and_then(|asset| asset.meta.height).unwrap_or(0);
        if height <= self.max_height {
            return Reason::SmallEnough;
        }
        match &decoded.proxy {
            None => Reason::Missing,
            Some(proxy) if !self.accepts(proxy, &frames.pattern) => Reason::Stale,
            Some(_) => Reason::Ready,
        }
    }

    /// A proxy belongs to the current policy and to the frames the asset is drawn from.
    pub fn accepts(self, proxy: &ProxyFrames, frames_pattern: &str) -> bool {
        proxy.height == self.max_height
            && proxy.format == self.format
            && proxy.source_frames == frames_pattern
    }

    /// The proxy the runtime may attach for `asset`, `None` when the policy says original.
    pub fn proxy_for(self, kind: MediaKind, asset: &Asset) -> Option<&ProxyFrames> {
        if self.assess(kind, Some(asset)) != Reason::Ready {
            return None;
        }
        asset.decoded.as_ref()?.proxy.as_ref()
    }

    /// Per-asset report for `preview.policy`.
    pub fn report(self, state: &EditorState) -> Value {
        let assets: Vec<Value> = state
            .library
            .items
            .iter()
            .filter(|item| item.kind == MediaKind::Video)
            .map(|item| {
                let reason = self.assess(item.kind, item.asset.as_ref());
                let proxy = item
                    .asset
                    .as_ref()
                    .and_then(|asset| asset.decoded.as_ref())
                    .and_then(|decoded| decoded.proxy.as_ref());
                json!({ "id": item.id, "reason": reason.label(),
                    "proxy": proxy.map(ProxyFrames::to_json),
                    "source_height": item.asset.as_ref().and_then(|asset| asset.meta.height) })
            })
            .collect();
        json!({ "policy": self.to_json(), "interactive_quality": self.interactive_quality().label(),
            "export": "original", "explicit_frames": "original", "assets": assets })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{Decoded, DecodedFrames, Identity, Meta, Source};

    fn video(height: u32, proxy: Option<ProxyFrames>) -> Asset {
        Asset {
            source: Source::File {
                path: "/x/a.mp4".into(),
            },
            meta: Meta {
                height: Some(height),
                ..Meta::default()
            },
            identity: Identity::default(),
            decoded: Some(Decoded {
                frames: Some(DecodedFrames {
                    pattern: "/c/f%06d.png".into(),
                    first: 0,
                    count: 10,
                }),
                audio: Vec::new(),
                proxy,
            }),
        }
    }

    fn proxy(height: u32, source: &str) -> ProxyFrames {
        ProxyFrames {
            pattern: "/c/p%06d.jpg".into(),
            first: 0,
            count: 10,
            width: height * 16 / 9,
            height,
            format: ProxyFormat::Jpeg,
            source_frames: source.into(),
        }
    }

    #[test]
    fn auto_wants_a_proxy_only_above_the_max_height() {
        let policy = PreviewPolicy::default();
        assert_eq!(
            policy.assess(MediaKind::Video, Some(&video(1080, None))),
            Reason::Missing
        );
        assert_eq!(
            policy.assess(MediaKind::Video, Some(&video(540, None))),
            Reason::SmallEnough
        );
        assert_eq!(
            policy.assess(MediaKind::Video, Some(&video(360, None))),
            Reason::SmallEnough
        );
    }

    #[test]
    fn original_mode_never_uses_a_proxy() {
        let policy = PreviewPolicy {
            mode: PreviewMode::Original,
            ..PreviewPolicy::default()
        };
        let asset = video(1080, Some(proxy(540, "/c/f%06d.png")));
        assert_eq!(
            policy.assess(MediaKind::Video, Some(&asset)),
            Reason::PolicyOriginal
        );
        assert!(policy.proxy_for(MediaKind::Video, &asset).is_none());
        assert_eq!(policy.interactive_quality(), Quality::Original);
    }

    #[test]
    fn a_proxy_of_other_frames_or_another_size_is_stale_not_used() {
        let policy = PreviewPolicy::default();
        let other_frames = video(1080, Some(proxy(540, "/other/f%06d.png")));
        assert_eq!(
            policy.assess(MediaKind::Video, Some(&other_frames)),
            Reason::Stale
        );
        assert!(policy.proxy_for(MediaKind::Video, &other_frames).is_none());
        let other_size = video(1080, Some(proxy(360, "/c/f%06d.png")));
        assert_eq!(
            policy.assess(MediaKind::Video, Some(&other_size)),
            Reason::Stale
        );
        let png = PreviewPolicy {
            format: ProxyFormat::Png,
            ..policy
        };
        let jpeg = video(1080, Some(proxy(540, "/c/f%06d.png")));
        assert_eq!(png.assess(MediaKind::Video, Some(&jpeg)), Reason::Stale);
    }

    #[test]
    fn a_matching_proxy_is_used_and_only_for_video() {
        let policy = PreviewPolicy::default();
        let asset = video(2160, Some(proxy(540, "/c/f%06d.png")));
        assert_eq!(policy.assess(MediaKind::Video, Some(&asset)), Reason::Ready);
        assert!(policy.proxy_for(MediaKind::Video, &asset).is_some());
        assert_eq!(
            policy.assess(MediaKind::Still, Some(&asset)),
            Reason::NotVideo
        );
    }

    #[test]
    fn a_video_without_frames_has_nothing_to_derive_from() {
        let mut asset = video(1080, None);
        asset.decoded = None;
        assert_eq!(
            PreviewPolicy::default().assess(MediaKind::Video, Some(&asset)),
            Reason::NoFrames
        );
    }

    #[test]
    fn the_policy_is_a_command_and_an_operation_that_is_not_an_edit() {
        use crate::headless;
        use crate::state::{Command, ProjectSettings};
        use serde_json::json;
        let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
        assert_eq!(state.preview_policy, PreviewPolicy::default());
        let revision = state.project.revision;
        let reply = headless::execute(
            &mut state,
            &json!({"op":"preview.policy","mode":"original","max_height":360,"format":"png"}),
        )
        .unwrap();
        assert_eq!(reply["value"]["mode"], "original");
        assert_eq!(
            state.preview_policy,
            PreviewPolicy {
                mode: PreviewMode::Original,
                max_height: 360,
                format: ProxyFormat::Png
            }
        );
        assert_eq!(
            state.project.revision, revision,
            "a viewing preference is not an edit"
        );
        assert!(!state.history.can_undo());
        // Without fields the operation reports the policy and why each video is (not) a proxy.
        let report = headless::execute(&mut state, &json!({"op":"preview.policy"})).unwrap();
        assert_eq!(report["policy"]["mode"], "original");
        assert_eq!(report["export"], "original");
        assert_eq!(report["interactive_quality"], "original");
        for bad in [
            json!({"op":"preview.policy","mode":"fast"}),
            json!({"op":"preview.policy","format":"gif"}),
            json!({"op":"preview.policy","max_height":50}),
        ] {
            assert!(headless::execute(&mut state, &bad).is_err(), "{bad}");
        }
        assert!(
            state
                .execute(Command::SetPreviewPolicy {
                    mode: None,
                    max_height: Some(99999),
                    format: None
                })
                .is_err()
        );
    }

    #[test]
    fn a_proxy_survives_re_decoding_the_same_frames_and_is_refused_for_other_ones() {
        use crate::state::{Command, ProjectSettings};
        let mut state = EditorState::empty(&ProjectSettings::default()).unwrap();
        state
            .execute(Command::AddAsset {
                id: Some("vid".into()),
                name: "Vid".into(),
                kind: MediaKind::Video,
                asset: Box::new(video(1080, Some(proxy(540, "/c/f%06d.png")))),
                folder: None,
            })
            .unwrap();
        let decoded = |state: &EditorState| {
            state
                .library
                .item("vid")
                .and_then(|item| item.asset.as_ref())
                .and_then(|asset| asset.decoded.clone())
                .unwrap()
        };
        // Preparing again lands the same frames without a proxy: the proxy stays.
        let mut again = decoded(&state);
        again.proxy = None;
        assert!(state.set_decoded("vid", Some(again.clone())));
        assert!(decoded(&state).proxy.is_some());
        // Other frames (the file changed): the proxy would show another picture, so it goes.
        let mut other = again;
        other.frames.as_mut().unwrap().pattern = "/other/f%06d.png".into();
        assert!(state.set_decoded("vid", Some(other)));
        assert!(decoded(&state).proxy.is_none());
        assert!(!state.set_preview_proxy("vid", Some(proxy(540, "/c/f%06d.png"))));
        assert!(!state.set_preview_proxy("missing", None));
    }
}
