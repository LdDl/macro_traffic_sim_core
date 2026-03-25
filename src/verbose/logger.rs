use crate::verbose::VerboseLevel;
use std::fmt;

/// Simple logger (per-pipeline) that does not depend on global variables.
#[derive(Debug, Clone)]
pub struct LocalLogger {
    level: VerboseLevel,
    pipeline_tag: Option<String>,
}

impl LocalLogger {
    pub fn new(level: VerboseLevel) -> Self {
        Self {
            level,
            pipeline_tag: None,
        }
    }

    pub fn none() -> Self {
        Self {
            level: VerboseLevel::None,
            pipeline_tag: None,
        }
    }

    pub fn with_pipeline(level: VerboseLevel, tag: impl Into<String>) -> Self {
        Self {
            level,
            pipeline_tag: Some(tag.into()),
        }
    }

    pub fn level(&self) -> VerboseLevel {
        self.level
    }

    pub fn set_level(&mut self, level: VerboseLevel) {
        self.level = level;
    }

    pub fn set_pipeline_tag(&mut self, tag: impl Into<String>) {
        self.pipeline_tag = Some(tag.into());
    }

    pub fn is_at_least(&self, min_level: VerboseLevel) -> bool {
        self.level >= min_level
    }

    pub fn log(&self, event: &str, message: &str) {
        if self.level == VerboseLevel::None {
            return;
        }
        self.print_line(event, message, &[])
    }

    pub fn log_with_fields(
        &self,
        event: &str,
        message: &str,
        fields: &[(&str, &dyn fmt::Display)],
    ) {
        if self.level == VerboseLevel::None {
            return;
        }
        self.print_line(event, message, fields)
    }

    fn print_line(&self, event: &str, message: &str, fields: &[(&str, &dyn fmt::Display)]) {
        use std::time::{SystemTime, UNIX_EPOCH};
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let mut buf = String::new();
        if let Some(tag) = &self.pipeline_tag {
            buf.push_str(&format!("\"pipeline\":\"{}\"", tag));
        }
        for (k, v) in fields {
            if !buf.is_empty() {
                buf.push_str(", ");
            }
            buf.push_str(&format!("\"{}\":\"{}\"", k, v));
        }
        println!(
            "{{\"ts\":{},\"level\":\"{}\",\"event\":\"{}\",\"msg\":\"{}\"{}{}}}",
            ts,
            self.level,
            event,
            message.replace('"', "'"),
            if buf.is_empty() { "" } else { "," },
            buf
        );
    }
}
