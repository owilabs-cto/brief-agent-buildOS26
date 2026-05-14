use anyhow::Result;
use std::{fs, path::Path, process::Command};
use tracing::warn;

#[derive(Clone, Debug)]
pub struct LoadedDoc {
    pub path: String,
    pub content: String,
}

const MARKDOWN_PATHS: &[&str] = &[
    "../CONTEXT-MAP.md",
    "../AGENTS.md",
    "../design-system.md",
    "../audit-agent/CLAUDE.md",
    "../audit-agent/CONTEXT.md",
    "../audit-agent/docs/adr/ADR-001-voice-provider-abstraction.md",
];

const PDF_PATHS: &[&str] = &[
    "../OWI_Audit_Storytelling_Note.pdf",
    "../OWI_Transcription_01_Direction.pdf",
    "../OWI_Transcription_02_Integration.pdf",
];

const DOCX_PATHS: &[&str] = &["../OWI Labs - Plan de croissance 90 jours.docx"];

pub fn load_all() -> Result<Vec<LoadedDoc>> {
    let mut out = Vec::new();

    for p in MARKDOWN_PATHS {
        match fs::read_to_string(p) {
            Ok(content) => out.push(LoadedDoc { path: (*p).to_string(), content }),
            Err(e) => warn!(path = p, error = %e, "skip markdown"),
        }
    }

    for p in PDF_PATHS {
        match extract_pdf(Path::new(p)) {
            Ok(content) => out.push(LoadedDoc { path: (*p).to_string(), content }),
            Err(e) => warn!(path = p, error = %e, "skip pdf"),
        }
    }

    for p in DOCX_PATHS {
        match extract_docx(Path::new(p)) {
            Ok(content) => out.push(LoadedDoc { path: (*p).to_string(), content }),
            Err(e) => warn!(path = p, error = %e, "skip docx"),
        }
    }

    Ok(out)
}

fn extract_pdf(path: &Path) -> Result<String> {
    let out = Command::new("pdftotext").arg(path).arg("-").output()?;
    if !out.status.success() {
        anyhow::bail!("pdftotext exit {:?}: {}", out.status.code(), String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn extract_docx(path: &Path) -> Result<String> {
    let out = Command::new("pandoc").arg("-t").arg("plain").arg(path).output()?;
    if !out.status.success() {
        anyhow::bail!("pandoc exit {:?}: {}", out.status.code(), String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
