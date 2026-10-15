//! Every hull's picture state: what it is drawn as, and what still has to be
//! fetched before it can be drawn.
//!
//! A MOVE, not a redesign (ADR-0020, step 1): `AssetKind`, `UnitVisual` and
//! `VisualCache` were `main.rs`'s, and their fifteen tests were pinned there
//! too. They are here because the rules are pure — manifest versions, presign
//! expiry, settled absence — and a rule that can only be checked by standing up
//! a store connection, a core map and a token is a rule that does not get
//! checked.
//!
//! The one type that changed is the decoded texture. It used to be an
//! `egui::load::SizedTexture` inside the visual, which is a toolkit type in the
//! middle of the model and is exactly what ADR-0020 is removing. The kernel now
//! holds a [`TextureHandle`]: an opaque number the SHELL mints when it has the
//! decoded bytes. egui keeps its `SizedTexture` beside it, GPUI will keep
//! whatever its image slot is (U6's `FrameSlot`), and neither name appears
//! here. What the kernel needs to know is only that a picture is held — which
//! is what `is_map_renderable` and `needs_url` ask.

use std::time::{Duration, Instant};

/// What kind of asset a visual carries.
///
/// Minos publishes this discriminator in the manifest so a future
/// archive kind can be added without making old clients guess. The
/// current `unit-images` bundle speaks `unit_image`; anything else is
/// retained as unsupported and never drawn as a hull photograph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    UnitImage,
    Unsupported,
    /// No asset for this unit in the active manifest version. This is
    /// settled absence, not an unresolved fetch.
    Unavailable,
}

impl AssetKind {
    pub fn from_manifest(value: &str) -> Self {
        match value {
            "unit_image" => AssetKind::UnitImage,
            _ => AssetKind::Unsupported,
        }
    }
}

/// Pause before asking again after a failed manifest or source read.
/// A null presign does not erase a real manifest entry, and a refused
/// request does not deserve an every-frame retry storm.
pub const IMAGE_READ_RETRY_DELAY: Duration = Duration::from_secs(30);
/// Re-read the manifest periodically so a picture added after the first

/// Read the standard S3 presign lifetime from the source URI when it
/// is present. A local archive path has no expiry and returns `None`.
pub fn image_source_expires_at(source: &str) -> Option<Instant> {
    let query = source.split_once('?')?.1;
    let seconds = query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .find(|(key, _)| *key == "X-Amz-Expires")?
        .1
        .parse::<u64>()
        .ok()?;
    Instant::now().checked_add(Duration::from_secs(seconds))
}

/// A decoded picture the shell is holding, addressed by the number it minted.
///
/// The kernel stores the FACT that a texture exists and never its identity in
/// toolkit terms: egui's `SizedTexture` and GPUI's image slot are both the
/// shell's business. `set_texture` takes one the shell made, and the shell
/// keeps its own map from this number back to whatever it must paint with —
/// the same flip as `view::Rgba::as_f32`, applied to a resource instead of a
/// colour.
///
/// `u64` because both toolkits address their texture tables with one, and
/// because nothing here may compare two handles as anything but opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextureHandle(u64);

impl TextureHandle {
    /// Wrap a handle the shell minted.
    pub fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// The number, for the shell's own lookup table.
    pub fn raw(self) -> u64 {
        self.0
    }
}

/// One unit's visual state: what it is drawn as, and everything the
/// renderer needs to draw it true-to-scale.
///
/// Deliberately NOT a `HashMap<i64, Option<String>>`. That could not
/// say whether a unit had no image or had not been fetched yet, could
/// not carry the measurements that make a thumbnail honest, and could
/// not survive a manifest change without a full rebuild. This is the
/// whole visual truth for one hull, in one value.
///
/// `image_url` is a PRESIGNED, EXPIRY-BEARING address: it lives here
/// in memory, never in SQLite, and `asset_version` (the manifest's
/// ETag) is what identifies the asset, not the URL.
#[derive(Clone)]
pub struct UnitVisual {
    pub unit_id: i64,
    /// Temporary source URI, valid only while its presign lives (or
    /// indefinitely for a later local archive path). None means no
    /// address is currently held.
    pub image_url: Option<String>,
    /// When a presigned source is known to expire. The texture is the
    /// durable result; the URL is only a way to obtain it.
    pub image_url_expires_at: Option<Instant>,
    /// A null presign is a failed read, not proof that the manifest
    /// entry was imaginary. Retry after a pause rather than every frame.
    pub image_url_retry_at: Option<Instant>,
    /// The decoded picture, once the shell has it. The kernel holds the
    /// handle, the shell holds the bytes; see [`TextureHandle`].
    pub texture: Option<TextureHandle>,
    /// Intrinsic dimensions as Minos measured them, before decoding.
    pub width_px: Option<u32>,
    pub height_px: Option<u32>,
    /// The manifest's declared media type. It is diagnostic metadata,
    /// not a second source: the map and Inspector both decode the same
    /// presigned object URL.
    pub content_type: String,
    /// Real-world measurements, straight from the published spec.
    /// None means unpublished; the renderer must not substitute.
    pub loa_m: Option<f64>,
    pub beam_m: Option<f64>,
    pub draft_m: Option<f64>,
    /// Which way this unit's image points, in compass degrees.
    /// The client constant today; backend metadata overrides it when
    /// Minos publishes orientation.
    pub forward_heading_deg: f32,
    /// The manifest ETag this visual was resolved against.
    /// A change invalidates the picture, not the unit's identity.
    pub asset_version: String,
    pub asset_kind: AssetKind,
    pub map_symbol: crate::store::MapSymbol,
}

impl std::fmt::Debug for UnitVisual {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Keep the source address redacted even though the decoded
        // texture identity is safe to name.
        f.debug_struct("UnitVisual")
            .field("unit_id", &self.unit_id)
            .field("image_url", &self.image_url.as_ref().map(|_| "<source>"))
            .field("url_expired", &self.image_url_expires_at.is_some_and(|at| at <= Instant::now()))
            .field("url_retry_pending", &self.image_url_retry_at)
            .field("texture", &self.texture.as_ref().map(|_| "<tex>"))
            .field("size_px", &(self.width_px, self.height_px))
            .field("content_type", &self.content_type)
            .field("loa_m", &self.loa_m)
            .field("beam_m", &self.beam_m)
            .field("forward_heading_deg", &self.forward_heading_deg)
            .field("asset_version", &self.asset_version)
            .field("asset_kind", &self.asset_kind)
            .field("map_symbol", &self.map_symbol)
            .finish()
    }
}

impl UnitVisual {
    /// A unit with no image: it draws a symbol, and the cache records
    /// that as a settled fact rather than an unanswered question.
    ///
    /// It still belongs to a manifest version. A hull that has no
    /// picture in v1 may have one in v2, so "absent" is versioned just
    /// like "present"; it is not a permanent property of the unit.
    pub fn unavailable(
        unit_id: i64,
        asset_version: &str,
        symbol: crate::store::MapSymbol,
    ) -> Self {
        UnitVisual {
            unit_id,
            image_url: None,
            image_url_expires_at: None,
            image_url_retry_at: None,
            texture: None,
            width_px: None,
            height_px: None,
            content_type: String::new(),
            loa_m: None,
            beam_m: None,
            draft_m: None,
            forward_heading_deg: crate::map_render::IMAGE_FORWARD_HEADING_DEG,
            asset_version: asset_version.to_string(),
            asset_kind: AssetKind::Unavailable,
            map_symbol: symbol,
        }
    }

    /// From a manifest entry, before any bytes are fetched. Intrinsic
    /// image dimensions and optional physical measurements are already
    /// known here and are carried exactly as Minos published them.
    pub fn from_entry(
        entry: &crate::backend::UnitImageEntry,
        asset_version: &str,
        symbol: crate::store::MapSymbol,
    ) -> Self {
        UnitVisual {
            unit_id: entry.unit_id,
            image_url: None,
            image_url_expires_at: None,
            image_url_retry_at: None,
            texture: None,
            width_px: entry.width_px,
            height_px: entry.height_px,
            content_type: entry.content_type.clone(),
            loa_m: entry.loa_m,
            beam_m: entry.beam_m,
            draft_m: None,
            forward_heading_deg: crate::map_render::IMAGE_FORWARD_HEADING_DEG,
            asset_version: asset_version.to_string(),
            asset_kind: AssetKind::from_manifest(&entry.asset_kind),
            map_symbol: symbol,
        }
    }

    /// True when this unit may be drawn as a map thumbnail. Requires
    /// a supported unit-image asset and a decoded texture — a
    /// renderer reaching a unit mid-fetch gets a symbol instead of a
    /// hole.
    pub fn is_map_renderable(&self) -> bool {
        self.asset_kind == AssetKind::UnitImage && self.texture.is_some()
    }
}

/// Cache key: the unit plus the manifest ETag that says
/// what its picture is. The URL is deliberately absent — a presigned
/// address expires and changes without changing the asset.
/// Cache key: the unit plus the manifest ETag that says
/// what its picture is.
pub type VisualKey = (i64, String);

/// Every unit's visual state for every asset version still held.
///
/// ONE cache serves both delivery modes: online it holds presigned
/// URLs, and the archive reader will later put extracted local paths
/// into the same entries. The key is `(unit_id, asset_version)`, so
/// the same hull under a new manifest can never inherit an old
/// picture by accident.
#[derive(Default)]
pub struct VisualCache {
    units: std::collections::HashMap<VisualKey, UnitVisual>,
    /// The manifest ETag currently selected by `get` and
    /// mutation helpers. A separate bool records that a manifest was
    /// read: an absent backend ETag must not become an invitation to
    /// fetch the manifest forever.
    asset_version: String,
    manifest_loaded: bool,
}

impl VisualCache {
    fn active_key(&self, unit_id: i64) -> VisualKey {
        (unit_id, self.asset_version.clone())
    }

    /// The visual for a unit at the active manifest version, if one
    /// has been resolved.
    pub fn get(&self, unit_id: i64) -> Option<&UnitVisual> {
        self.units.get(&self.active_key(unit_id))
    }

    /// True when a manifest has been read at all. An absent backend ETag
    /// must not become an invitation to fetch the manifest forever, which is
    /// why this is a method beside the field rather than the field itself.
    pub fn manifest_loaded(&self) -> bool {
        self.manifest_loaded
    }

    /// True when the unit's state is settled. An absent picture is a
    /// real `UnitVisual` with `AssetKind::Unavailable`, so this also
    /// distinguishes "the manifest says no" from "not fetched yet".
    pub fn is_settled(&self, unit_id: i64) -> bool {
        self.units.contains_key(&self.active_key(unit_id))
    }

    /// True while this visual can still benefit from acquiring its
    /// unit-image source. Timing (live, retrying, or due) is separate.
    pub fn expects_source(&self, unit_id: i64) -> bool {
        self.get(unit_id).is_some_and(|v| {
            v.asset_kind == AssetKind::UnitImage && v.texture.is_none()
        })
    }

    /// Bound retries after a transport failure without changing the
    /// manifest's claim that this unit has a picture.
    pub fn defer_source(&mut self, unit_id: i64) {
        if let Some(v) = self.units.get_mut(&self.active_key(unit_id)) {
            v.image_url_retry_at = Some(Instant::now() + IMAGE_READ_RETRY_DELAY);
        }
    }

    /// True when a unit-image still needs a usable source address.
    /// A loaded texture needs no URL, unsupported and absent assets do
    /// not, and a null presign waits for a bounded retry.
    pub fn needs_url(&self, unit_id: i64) -> bool {
        let Some(visual) = self.get(unit_id) else {
            return false;
        };
        if visual.asset_kind != AssetKind::UnitImage || visual.texture.is_some() {
            return false;
        }
        if visual
            .image_url_retry_at
            .is_some_and(|retry_at| retry_at > Instant::now())
        {
            return false;
        }
        match &visual.image_url {
            Some(_) => visual
                .image_url_expires_at
                .is_some_and(|expires_at| expires_at <= Instant::now()),
            None => true,
        }
    }

    /// Record a unit as having no picture in the active manifest.
    /// Cached under that version, so the manifest is not re-asked on
    /// every frame for a hull that will never have one.
    pub fn mark_absent(&mut self, unit_id: i64, symbol: crate::store::MapSymbol) {
        let version = self.asset_version.clone();
        let mut visual = UnitVisual::unavailable(unit_id, &version, symbol);
        if let Some(previous) = self.get(unit_id) {
            // The specification did not change merely because the
            // image did. Preserve unpublished-versus-published truth.
            visual.loa_m = previous.loa_m;
            visual.beam_m = previous.beam_m;
            visual.draft_m = previous.draft_m;
        }
        self.units.insert((unit_id, version), visual);
    }

    /// Install the manifest: every listed unit gets a visual seeded
    /// from its entry, and the version stamps them all.
    ///
    /// Returns the units whose URL still needs reading. Re-installing
    /// the same version is idempotent, including live URLs and loaded
    /// textures. A version change starts every picture again, while
    /// preserving the physical measurements, which belong to the unit
    /// specification rather than to the photograph.
    pub fn install_manifest(
        &mut self,
        manifest: &crate::backend::ImageManifest,
        symbol_of: impl Fn(i64) -> crate::store::MapSymbol,
    ) -> Vec<i64> {
        let same_version = self.manifest_loaded
            && !manifest.version.is_empty()
            && self.asset_version == manifest.version;
        // Snapshot before clearing: measurements belong to the unit,
        // so they survive a picture-version change even though URLs and
        // textures do not.
        let previous: std::collections::HashMap<i64, UnitVisual> =
            if self.manifest_loaded && !same_version {
                self.units
                    .iter()
                    .filter_map(|((id, _), visual)| Some((*id, visual.clone())))
                    .collect()
            } else {
                std::collections::HashMap::new()
            };
        if !same_version {
            // Old pictures and old absence are facts about another
            // asset version. Do not let a lookup by unit id find them.
            self.units.clear();
        }
        self.asset_version = manifest.version.clone();
        self.manifest_loaded = true;

        let mut needing_url = Vec::new();
        for entry in &manifest.entries {
            let key = (entry.unit_id, manifest.version.clone());
            if self.units.contains_key(&key) {
                // Same content version: keep the live URL, texture,
                // and dimensions. Re-reading the manifest is not a
                // reason to refetch every picture.
                continue;
            }
            let mut visual =
                UnitVisual::from_entry(entry, &manifest.version, symbol_of(entry.unit_id));
            if let Some(old) = previous.get(&entry.unit_id) {
                // A manifest may carry the active physical measurements
                // even when the local specification mirror does not. A
                // missing value in the new manifest must not erase the
                // last known value across an asset-version refresh.
                visual.loa_m = visual.loa_m.or(old.loa_m);
                visual.beam_m = visual.beam_m.or(old.beam_m);
                visual.draft_m = visual.draft_m.or(old.draft_m);
            }
            let needs_source = visual.asset_kind == AssetKind::UnitImage;
            self.units.insert(key, visual);
            if needs_source {
                needing_url.push(entry.unit_id);
            }
        }
        needing_url
    }

    /// Merge the unit facts that come from the local mirror. These
    /// are not picture bytes: measurements, taxonomy-derived symbol,
    /// and their version. A unit absent from the manifest gets a real
    /// `Unavailable` visual rather than a side-table exception, so all
    /// three renderer states travel through one model.
    pub fn set_unit_facts(
        &mut self,
        unit_id: i64,
        symbol: crate::store::MapSymbol,
        loa_m: Option<f64>,
        beam_m: Option<f64>,
        draft_m: Option<f64>,
    ) {
        if !self.manifest_loaded {
            return;
        }
        let key = self.active_key(unit_id);
        let version = self.asset_version.clone();
        let visual = self
            .units
            .entry(key)
            .or_insert_with(|| UnitVisual::unavailable(unit_id, &version, symbol));
        visual.map_symbol = symbol;
        // The manifest is the current Minos truth when it carries a
        // measurement; the local mirror only fills a gap. A stale local
        // spec must not overwrite a value that arrived with this asset
        // version. A missing local row is not evidence that Minos
        // withdrew the manifest's measurement.
        if visual.loa_m.is_none() {
            visual.loa_m = loa_m;
        }
        if visual.beam_m.is_none() {
            visual.beam_m = beam_m;
        }
        if draft_m.is_some() {
            visual.draft_m = draft_m;
        }
    }

    /// Replace a unit's temporary URL, keeping every other fact. This
    /// is the presigned-expiry path: the same asset, a new address.
    pub fn set_url(&mut self, unit_id: i64, url: Option<String>) {
        if let Some(v) = self.units.get_mut(&self.active_key(unit_id)) {
            v.image_url_expires_at = url.as_deref().and_then(image_source_expires_at);
            v.image_url_retry_at = url
                .is_none()
                .then(|| Instant::now() + IMAGE_READ_RETRY_DELAY);
            v.image_url = url;
        }
    }

    /// Install a decoded texture, by the handle the shell minted for it.
    pub fn set_texture(&mut self, unit_id: i64, handle: TextureHandle) {
        if let Some(v) = self.units.get_mut(&self.active_key(unit_id)) {
            v.texture = Some(handle);
        }
    }

    /// Drop everything. Called on sign-out, user change, new session
    /// and a held-game change: no image, texture, URL, or measurement
    /// from one user's exercise may survive into the next one's.
    pub fn clear(&mut self) {
        self.units.clear();
        self.asset_version.clear();
        self.manifest_loaded = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(unit_id: i64, w: Option<u32>, h: Option<u32>) -> crate::backend::UnitImageEntry {
        crate::backend::UnitImageEntry {
            asset_kind: "unit_image".to_string(),
            unit_id,
            name: format!("hull {unit_id}"),
            hull_number: None,
            file_name: format!("images/{unit_id}.png"),
            content_type: "image/png".to_string(),
            size_bytes: 1024,
            width_px: w,
            height_px: h,
            loa_m: None,
            beam_m: None,
        }
    }

    fn manifest(version: &str, entries: Vec<crate::backend::UnitImageEntry>) -> crate::backend::ImageManifest {
        crate::backend::ImageManifest {
            version: version.to_string(),
            entry_count: entries.len(),
            units_without_image: 0,
            entries,
        }
    }

    fn symbol(_: i64) -> crate::store::MapSymbol {
        crate::store::MapSymbol::Corvette
    }

    /// The manifest's discriminator is the asset kind. Pixel size is
    /// carried honestly but does not silently invent a second kind in
    /// this model ticket; thumbnail validation belongs to the archive
    /// reader's own contract.
    #[test]
    fn manifest_asset_kind_is_preserved_without_guessing() {
        let normal = UnitVisual::from_entry(&entry(1, Some(512), Some(256)), "v1", symbol(1));
        assert_eq!(normal.asset_kind, AssetKind::UnitImage);
        assert_eq!(normal.width_px, Some(512));
        assert_eq!(normal.height_px, Some(256));
        assert!(!normal.is_map_renderable(), "dimensions alone are not a texture");

        let mut future = entry(2, Some(1), Some(1));
        future.asset_kind = "account_portrait".to_string();
        let unsupported = UnitVisual::from_entry(&future, "v1", symbol(2));
        assert_eq!(unsupported.asset_kind, AssetKind::Unsupported);
        assert_eq!(unsupported.width_px, Some(1), "size is still reported as sent");
    }

    /// Physical measurements can arrive in the manifest even when the
    /// local spec mirror has not been populated. The manifest is a
    /// complete visual input; hydration must supplement it, never erase
    /// it with a missing local value.
    #[test]
    fn manifest_measurements_survive_local_hydration() {
        let mut published = entry(1, Some(512), Some(256));
        published.loa_m = Some(120.5);
        published.beam_m = Some(16.2);
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![published]), symbol);
        c.set_unit_facts(1, crate::store::MapSymbol::Corvette, None, None, None);

        let visual = c.get(1).expect("visual");
        assert_eq!(visual.loa_m, Some(120.5));
        assert_eq!(visual.beam_m, Some(16.2));
    }

    /// A local mirror fills a manifest gap, but does not overwrite a
    /// current value that Minos already published in the manifest.
    #[test]
    fn local_measurements_fill_only_manifest_gaps() {
        let mut published = entry(1, Some(512), Some(256));
        published.loa_m = Some(120.5);
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![published]), symbol);
        c.set_unit_facts(1, crate::store::MapSymbol::Corvette, Some(130.0), Some(18.0), None);

        let visual = c.get(1).expect("visual");
        assert_eq!(visual.loa_m, Some(120.5), "manifest is authoritative");
        assert_eq!(visual.beam_m, Some(18.0), "local mirror fills the missing beam");
    }

    /// A listed unit image needs a source; a future asset kind and a
    /// versioned absence do not. A null presign waits before retrying
    /// rather than hammering the API every frame, and an expired AWS
    /// presign becomes eligible again.
    #[test]
    fn source_resolution_distinguishes_absent_failed_and_expired() {
        let mut c = VisualCache::default();
        let mut future = entry(2, Some(32), Some(32));
        future.asset_kind = "account_portrait".to_string();
        let queued = c.install_manifest(
            &manifest("v1", vec![entry(1, Some(512), Some(256)), future]),
            symbol,
        );
        assert_eq!(queued, vec![1], "future kinds never enter the source queue");
        c.mark_absent(3, crate::store::MapSymbol::Submarine);
        assert!(c.needs_url(1));
        assert!(!c.needs_url(2), "an unsupported future kind is not a hull image");
        assert!(!c.needs_url(3), "absence has no file to address");

        c.set_url(1, None);
        assert!(!c.needs_url(1), "a null presign waits for the retry delay");
        c.set_url(1, Some("https://objects/one.png?X-Amz-Expires=60".into()));
        assert!(!c.needs_url(1), "a live presign needs no retry");
        c.set_url(1, Some("https://objects/one.png?X-Amz-Expires=0".into()));
        assert!(c.needs_url(1), "an expired presign is read again");
    }

    /// A unit with no image, a unit not yet fetched, and a unit whose
    /// bytes are still arriving are three different states. The old
    /// Option<String> map could not tell them apart.
    #[test]
    fn settled_absence_differs_from_unfetched() {
        let mut c = VisualCache::default();
        assert!(!c.is_settled(7), "manifest not read yet");
        c.install_manifest(&manifest("v1", vec![]), symbol);
        assert!(!c.is_settled(7), "manifest read, unit not resolved");
        c.mark_absent(7, crate::store::MapSymbol::Auxiliary);
        assert!(c.is_settled(7), "asked, and the answer was no image");
        let absent = c.get(7).expect("absence is a real visual");
        assert_eq!(absent.asset_kind, AssetKind::Unavailable);
        assert_eq!(absent.map_symbol, crate::store::MapSymbol::Auxiliary);
        assert_eq!(absent.asset_version, "v1", "absence is versioned too");
        assert!(!c.is_settled(8), "a different unit is still open");
    }

    /// A new URL is the same asset. Identity is the unit and the
    /// version, never the address, so an expiring presigned URL is
    /// replaced without disturbing anything else.
    #[test]
    fn url_replacement_preserves_the_asset() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_unit_facts(
            1,
            crate::store::MapSymbol::Corvette,
            Some(120.0),
            Some(16.0),
            Some(4.0),
        );
        c.set_url(1, Some("https://objects/one?sig=a".into()));
        let before = c.get(1).expect("visual");
        assert_eq!(before.asset_version, "v1");
        assert_eq!(before.loa_m, Some(120.0));

        c.set_url(1, Some("https://objects/one?sig=b".into()));
        let after = c.get(1).expect("visual");
        assert_eq!(after.image_url.as_deref(), Some("https://objects/one?sig=b"));
        assert_eq!(after.asset_version, "v1", "version is not the URL's job");
        assert_eq!(after.loa_m, Some(120.0), "measurements survive the swap");
        assert_eq!(after.map_symbol, crate::store::MapSymbol::Corvette);
    }

    /// A manifest version change means the assets themselves moved.
    /// Pictures are re-resolved; the unit ids and their identity are
    /// untouched, because the units did not change.
    #[test]
    fn version_change_invalidates_pictures_not_units() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_url(1, Some("https://objects/one?sig=a".into()));
        assert_eq!(c.get(1).expect("v1 visual").asset_version, "v1");

        // Same units, new version: the old URL is not carried over.
        let needing = c.install_manifest(&manifest("v2", vec![entry(1, Some(640), Some(320))]), symbol);
        assert_eq!(needing, vec![1], "the URL must be re-read for v2");
        let v2 = c.get(1).expect("v2 visual");
        assert_eq!(v2.asset_version, "v2");
        assert_eq!(v2.image_url, None, "a v1 URL does not describe a v2 asset");
        assert_eq!(v2.width_px, Some(640), "new dimensions applied");
        assert!(!c.units.contains_key(&(1, "v1".to_string())));
        assert!(c.units.contains_key(&(1, "v2".to_string())));
    }

    /// A unit that had a picture in one version and is omitted from
    /// the next is not silently absent: it becomes unresolved under
    /// v2, so selection can record fresh versioned absence rather than
    /// inheriting the v1 answer.
    #[test]
    fn omission_in_a_new_version_clears_old_absence() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![]), symbol);
        c.mark_absent(7, crate::store::MapSymbol::Destroyer);
        assert!(c.is_settled(7));

        c.install_manifest(&manifest("v2", vec![]), symbol);
        assert!(!c.is_settled(7), "v1 absence cannot answer for v2");
        assert!(!c.units.contains_key(&(7, "v1".to_string())));
    }

    /// The backend's ETag may be absent. That still records that a
    /// manifest was loaded, but an unknown version never claims cache
    /// identity or preserves a URL across a repeated read.
    #[test]
    fn empty_content_stamp_is_still_a_loaded_version() {
        let mut c = VisualCache::default();
        c.install_manifest(
            &manifest("", vec![entry(1, Some(512), Some(256))]),
            symbol,
        );
        assert!(c.manifest_loaded);
        assert!(c.is_settled(1));
        assert_eq!(c.get(1).expect("visual").asset_version, "");
        c.set_url(1, Some("https://objects/one".into()));
        let needing = c.install_manifest(
            &manifest("", vec![entry(1, Some(512), Some(256))]),
            symbol,
        );
        assert_eq!(needing, vec![1], "an unknown version cannot claim cache identity");
        assert_eq!(c.get(1).expect("visual").image_url, None);
    }

    /// Installing the same version twice is idempotent and asks for
    /// nothing, so a re-read does not re-fetch every URL.
    #[test]
    fn reinstalling_the_same_version_asks_for_nothing() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_url(1, Some("https://objects/one?sig=a".into()));
        let needing = c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        assert!(needing.is_empty(), "same version, no new URLs wanted");
        assert_eq!(
            c.get(1).expect("visual").image_url.as_deref(),
            Some("https://objects/one?sig=a"),
            "and the live URL is kept"
        );
    }

    /// Measurements live alongside pictures, not inside them, and
    /// survive an asset change because the specification did not
    /// change with the photograph.
    #[test]
    fn dimensions_outlive_the_picture() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_unit_facts(
            1,
            crate::store::MapSymbol::Corvette,
            Some(120.5),
            Some(16.2),
            Some(4.1),
        );
        c.install_manifest(&manifest("v2", vec![entry(1, Some(512), Some(256))]), symbol);
        let v = c.get(1).expect("visual");
        assert_eq!(v.loa_m, Some(120.5));
        assert_eq!(v.beam_m, Some(16.2));
        assert_eq!(v.draft_m, Some(4.1));
    }

    /// A picture added after the first manifest read replaces the
    /// versioned absence and enters the source queue.
    #[test]
    fn later_manifest_promotes_unavailable_to_a_picture() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![]), symbol);
        c.mark_absent(351, crate::store::MapSymbol::Destroyer);
        assert!(!c.needs_url(351));

        let needing = c.install_manifest(
            &manifest("v2", vec![entry(351, Some(512), Some(256))]),
            symbol,
        );
        assert_eq!(needing, vec![351]);
        assert_eq!(c.get(351).expect("new visual").asset_kind, AssetKind::UnitImage);
        assert!(c.needs_url(351));
    }

    /// Unpublished measurements stay unpublished, so the renderer can
    /// tell "unknown" from "zero metres".
    #[test]
    fn unpublished_measurements_read_as_unknown() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_unit_facts(1, crate::store::MapSymbol::Corvette, None, None, None);
        let v = c.get(1).expect("visual");
        assert_eq!(v.loa_m, None);
        assert_eq!(v.beam_m, None);
        // …and a hull with no image is still drawable, as a symbol.
        let absent = UnitVisual::unavailable(99, "v1", crate::store::MapSymbol::Auxiliary);
        assert_eq!(absent.asset_kind, AssetKind::Unavailable);
        assert_eq!(absent.map_symbol, crate::store::MapSymbol::Auxiliary);
    }

    /// The session boundary: nothing survives it. This is the
    /// sign-out / user-change guarantee.
    #[test]
    fn clear_leaves_nothing_behind() {
        let mut c = VisualCache::default();
        c.install_manifest(&manifest("v1", vec![entry(1, Some(512), Some(256))]), symbol);
        c.set_url(1, Some("https://objects/one".into()));
        c.set_unit_facts(
            1,
            crate::store::MapSymbol::Corvette,
            Some(120.0),
            Some(16.0),
            None,
        );
        c.mark_absent(2, crate::store::MapSymbol::Destroyer);
        c.clear();
        assert!(c.units.is_empty(), "no unit keeps a URL or measurement");
        assert!(c.asset_version.is_empty(), "no version survives");
        assert!(!c.manifest_loaded, "the next session must read its manifest");
    }

    /// A map thumbnail needs BOTH a supported unit-image asset and a
    /// decoded texture. A unit mid-fetch is not renderable, so a
    /// renderer reaching it draws a symbol rather than a hole.
    #[test]
    fn map_renderability_needs_a_texture() {
        let v = UnitVisual::from_entry(&entry(1, Some(512), Some(256)), "v1", symbol(1));
        assert!(!v.is_map_renderable(), "metadata alone are not a texture");
        let mut future = entry(2, Some(512), Some(256));
        future.asset_kind = "account_portrait".to_string();
        let unsupported = UnitVisual::from_entry(&future, "v1", symbol(2));
        assert!(!unsupported.is_map_renderable(), "a future kind is not a hull image");
    }
}
