//! Getting started: the setup checklist, cloning a GitHub repository into a new folder, onboarding progress, and
//! Home updates on a Hugging Face Space.

pub mod home_update;
pub mod onboarding;
pub mod project_clone;
pub mod setup_checklist;

use anyhow::Result;

/// Writes the TypeScript versions of this crate's types shared with the UI (ts-rs).
pub fn gen_types(cfg: &ts_rs::Config) -> Result<()> {
    use ts_rs::TS;
    project_clone::CloneSuggestion::export_all(cfg)?;
    onboarding::OnboardingState::export_all(cfg)?;
    setup_checklist::Checklist::export_all(cfg)?;
    setup_checklist::ChecklistSummary::export_all(cfg)?;
    home_update::HomeUpdate::export_all(cfg)?;
    Ok(())
}
