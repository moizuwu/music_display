//! The one-time question before lyrics go to an AI provider that is not on this machine. It is
//! the terminal UI's notice (same wording, and the same `ai-notice-ack` file, so answering in
//! either front end counts for both), but here it is a real gate: with the AI engine or the AI
//! annotations chosen, nothing is sent until "Send" is pressed. The gate itself is plain logic
//! (unit-tested); the window at the bottom only asks.
use super::model::Model;
use music_display::ai;
use music_display::cli::{Args, TranslateEngine};
use std::path::PathBuf;

pub fn ack_path() -> PathBuf {
    music_display::spotify::dir().with_file_name("ai-notice-ack")
}

fn cfg(args: &Args) -> ai::Cfg {
    let provider = ai::Provider::parse(&args.ai_provider).unwrap_or(ai::Provider::OpenAiCompatible);
    ai::Cfg::new(provider, &args.ai_model, &args.ai_endpoint, &args.ai_key_env)
}

/// Would AI requests leave this machine without the user having been told? A local endpoint
/// (Ollama on localhost) never needs the notice, and with no model set nothing is sent at all.
pub fn notice_needed(args: &Args, acked: bool) -> bool {
    !acked && !args.ai_model.trim().is_empty() && !cfg(args).is_local()
}

pub fn notice(args: &Args) -> String {
    let c = cfg(args);
    let provider = ai::Provider::parse(&args.ai_provider).unwrap_or(ai::Provider::OpenAiCompatible);
    format!(
        "The AI features send the lyrics and song info (title, artist, album, genre, year) to {} ({})",
        c.endpoint,
        provider.name()
    )
}

fn ack() {
    let p = ack_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, "shown\n");
}

impl Model {
    /// True (and the question opens) when an AI request would go out before the notice was
    /// acknowledged. Callers then do not send.
    pub fn ai_blocked(&mut self) -> bool {
        if notice_needed(&self.args, ack_path().exists()) {
            self.ux.ai_ask = true;
            true
        } else {
            false
        }
    }

    /// "Send": remember the answer (for both front ends) and start what was waiting.
    pub fn ai_allow(&mut self) {
        ack();
        self.ux.ai_ask = false;
        self.translations.clear();
        self.translate_inflight = false;
        self.ensure_translation();
        self.ensure_annotations();
    }

    /// "Keep AI off": back to the standard translation engine and Genius-only notes.
    pub fn ai_decline(&mut self) {
        self.ux.ai_ask = false;
        let mut cfg: Vec<(&str, toml::Value)> = Vec::new();
        if self.args.translate_engine == TranslateEngine::Ai {
            self.args.translate_engine = TranslateEngine::Standard;
            cfg.push(("translate_engine", self.args.translate_engine.name().into()));
            self.translations.clear();
            self.translate_inflight = false;
        }
        if self.args.annotations_mode.uses_ai() {
            self.args.annotations_mode = music_display::cli::AnnotationMode::Genius;
            cfg.push(("annotations_mode", self.args.annotations_mode.name().into()));
            self.restart_annotations();
        }
        self.persist("AI off", &cfg);
        self.ensure_translation();
    }
}

/// The question, as a window that stays until answered.
pub fn window(ctx: &egui::Context, m: &mut Model) {
    if !m.ux.ai_ask {
        return;
    }
    let text = notice(&m.args);
    let (mut allow, mut decline) = (false, false);
    egui::Window::new("Send lyrics to an AI provider?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        ui.set_max_width(380.0);
        ui.add(egui::Label::new(text).wrap());
        ui.label(egui::RichText::new("Asked once; the answer is shared with the terminal UI. A local endpoint (Ollama on localhost) never asks.").small().weak());
        ui.horizontal(|ui| {
            allow = ui.button("Send").clicked();
            decline = ui.button("Keep AI off").clicked();
        });
    });
    if allow {
        m.ai_allow();
    } else if decline {
        m.ai_decline();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn args(extra: &[&str]) -> Args {
        let mut v = vec!["music_display"];
        v.extend_from_slice(extra);
        Args::parse_from(v)
    }

    #[test]
    fn a_remote_provider_needs_the_notice_until_it_is_acknowledged() {
        let a = args(&["--ai-provider", "openai", "--ai-model", "gpt"]);
        assert!(notice_needed(&a, false));
        assert!(!notice_needed(&a, true));
        assert!(notice(&a).contains("api.openai.com"));
    }

    #[test]
    fn a_local_endpoint_or_no_model_never_asks() {
        let local = args(&["--ai-model", "llama3", "--ai-endpoint", "http://localhost:11434/v1"]);
        assert!(!notice_needed(&local, false));
        let nomodel = args(&["--ai-provider", "claude"]);
        assert!(!notice_needed(&nomodel, false));
    }
}
