//! The annotations panel (experimental): what the active lyric line means. Notes come from
//! Genius (free client token) or, as a second mode, from the AI engine; either way they are
//! fetched in a task, tagged with the track, and matched to the lyric lines here.
use super::*;
use crate::annotations::{self, Annotation};
use crate::cli::AnnotationMode;
use crate::layout::PanelId;

pub enum AnnState {
    Idle,
    Loading,
    Ready,
    Failed(String),
}

impl App {
    pub fn annotations_shown(&self) -> bool {
        self.layout.panels.shown(PanelId::Annotations)
    }

    /// The Genius token from the config / flag, else `GENIUS_TOKEN`.
    pub fn genius_token(&self) -> Option<String> {
        annotations::token(&self.args.genius_token)
    }

    /// Index of the lyric line being sung (synced lyrics only).
    pub fn active_line(&self) -> Option<usize> {
        match &self.lyrics {
            LyricsState::Ready(Lyrics::Synced(l)) => {
                let t = self.position() + self.offset;
                l.partition_point(|(ts, _)| *ts <= t).checked_sub(1)
            }
            _ => None,
        }
    }

    /// The note for lyric line `i`: its text, the quoted fragment (Genius) and who wrote it.
    pub fn note_for(&self, i: usize) -> Option<Annotation> {
        match self.args.annotations_mode {
            AnnotationMode::Genius => {
                let a = self.anns.get((*self.ann_match.get(i)?)?)?;
                Some(a.clone())
            }
            AnnotationMode::Ai => {
                let text = self.ai_notes.get(i)?.clone()?;
                Some(Annotation { fragment: String::new(), text, source: annotations::AI_LABEL.into(), url: None })
            }
        }
    }

    /// Fetch the notes for this track once, while the panel is on and the lyrics are there.
    pub fn ensure_annotations(&mut self) {
        if !self.annotations_shown() || !matches!(self.ann_state, AnnState::Idle) {
            return;
        }
        let Some(lines) = self.lyric_lines().filter(|l| !l.is_empty()) else { return };
        let Some(snap) = self.snap.clone() else { return };
        let (c, tx, key) = (self.client.clone(), self.tx.clone(), self.track_key);
        match self.args.annotations_mode {
            AnnotationMode::Genius => {
                let Some(token) = self.genius_token() else {
                    self.ann_state = AnnState::Failed("no token: set genius_token or GENIUS_TOKEN (free at genius.com/api-clients)".into());
                    return;
                };
                self.ann_state = AnnState::Loading;
                let base = self.annotations_base.clone();
                self.tasks.push(tokio::spawn(async move {
                    let base = base.unwrap_or_else(|| annotations::API.to_string());
                    let result = annotations::fetch_from(&c, &base, &token, &snap.artist, &snap.title).await;
                    let _ = tx.send(AppEvent::Annotations { key, result });
                }));
            }
            AnnotationMode::Ai => {
                let Some(cfg) = self.ai_cfg() else {
                    self.ann_state = AnnState::Failed("AI notes need a model (settings → AI model)".into());
                    return;
                };
                if self.ai_notice_needed() {
                    self.set_note(self.ai_notice());
                    self.ack_ai_notice();
                }
                self.ann_state = AnnState::Loading;
                self.tasks.push(tokio::spawn(async move {
                    let result = annotations::ai_notes(&c, &cfg, &snap.artist, &snap.title, &lines).await;
                    let _ = tx.send(AppEvent::AiNotes { key, result });
                }));
            }
        }
    }

    pub(super) fn reset_annotations(&mut self) {
        self.ann_state = AnnState::Idle;
        self.anns.clear();
        self.ann_match.clear();
        self.ai_notes.clear();
    }

    /// Match the Genius fragments to the current lyric lines.
    pub fn rematch_annotations(&mut self) {
        self.ann_match = match self.lyric_lines() {
            Some(lines) => annotations::match_lines(&lines, &self.anns),
            None => Vec::new(),
        };
    }

    pub(super) fn on_annotations(&mut self, result: Result<Vec<Annotation>, String>) {
        match result {
            Ok(a) => {
                self.anns = a;
                self.rematch_annotations();
                self.ann_state = AnnState::Ready;
            }
            Err(e) => self.ann_state = AnnState::Failed(e),
        }
        self.dirty = true;
    }

    pub(super) fn on_ai_notes(&mut self, result: Result<Vec<Option<String>>, String>) {
        match result {
            Ok(n) => {
                self.ai_notes = n;
                self.ann_state = AnnState::Ready;
            }
            Err(e) => self.ann_state = AnnState::Failed(e),
        }
        self.dirty = true;
    }

    /// `N`: show / hide the panel.
    pub fn toggle_annotations(&mut self) {
        let on = !self.annotations_shown();
        self.layout.panels.set(PanelId::Annotations, on);
        self.args.annotations = on;
        self.snap_scroll = true;
        if on {
            self.ensure_annotations();
        }
    }

    /// The mode or the token changed: start over for this song.
    pub fn restart_annotations(&mut self) {
        self.reset_annotations();
        self.ensure_annotations();
    }
}
