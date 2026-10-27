//! Tags read straight from a local audio file (ID3, Vorbis/FLAC, MP4…), for players
//! whose MPRIS metadata is incomplete.
use lofty::file::TaggedFileExt;
use lofty::tag::Accessor;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
    /// `YYYY` (tags rarely carry more).
    pub year: String,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    /// `file://` URL of the embedded cover, written to the cache folder.
    pub art_url: String,
}

/// `file:///home/me/My%20Song.flac` → the path.
pub fn path_of(url: &str) -> Option<PathBuf> {
    crate::util::path_from_url(url)
}

/// File name without folder and extension — what to show when there are no tags at all.
pub fn title_from_url(url: &str) -> String {
    path_of(url)
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// Blocking: call from `spawn_blocking`.
pub fn read(url: &str) -> Option<Tags> {
    let path = path_of(url)?;
    let file = lofty::read_from_path(&path).ok()?;
    let tag = file.primary_tag().or_else(|| file.first_tag())?;
    let s = |v: Option<std::borrow::Cow<'_, str>>| v.map(|c| c.trim().to_string()).unwrap_or_default();
    let art_url = tag
        .pictures()
        .first()
        .and_then(|pic| {
            let p = crate::util::cache_dir().join(format!("tagart-{:016x}", crate::util::fnv1a(url)));
            std::fs::create_dir_all(p.parent()?).ok()?;
            std::fs::write(&p, pic.data()).ok()?;
            Some(crate::util::file_url(&p))
        })
        .unwrap_or_default();
    Some(Tags {
        title: s(tag.title()),
        artist: s(tag.artist()),
        album: s(tag.album()),
        year: tag.date().map(|d| d.year.to_string()).unwrap_or_default(),
        track: tag.track(),
        disc: tag.disk(),
        art_url,
    })
}
