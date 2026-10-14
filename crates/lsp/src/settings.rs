//! The client's settings: elm-language-server's `elmLS` section, as the
//! initialization options, a configuration change or the answer to
//! `workspace/configuration` give it.

use nomi_ide::{CheckSettings, ReviewSettings, Severity};
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Settings {
    /// What diagnostics check.
    pub(crate) checks: CheckSettings,
    /// Whether a package is installed without asking first.
    pub(crate) skip_install_confirmation: bool,
    pub(crate) extended_capabilities: ExtendedCapabilities,
}

/// `IExtendedCapabilites`: what the client does beyond the protocol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExtendedCapabilities {
    /// Whether the client asks where to move a function, and moves it.
    pub(crate) move_function: bool,
    /// Whether the client runs `elm.expose-` and `elm.unexpose-`
    /// commands, which lenses then offer.
    pub(crate) expose_commands: bool,
}

impl Settings {
    /// Reads `value`: the `elmLS` section, or settings that hold it.
    pub(crate) fn read(value: &Value) -> Option<Settings> {
        let section = value.get("elmLS").unwrap_or(value).as_object()?;
        let flag = |name: &str| section.get(name).and_then(Value::as_bool).unwrap_or(false);
        let extended = |name: &str| {
            (section.get("extendedCapabilities"))
                .and_then(|e| e.get(name))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        let path = |name: &str| {
            let path = section.get(name).and_then(Value::as_str)?.trim();
            (!path.is_empty()).then(|| path.to_owned())
        };
        let severity = match section.get("elmReviewDiagnostics").and_then(Value::as_str) {
            Some("warning") => Some(Severity::Warning),
            Some("error") => Some(Severity::Error),
            _ => None,
        };
        Some(Settings {
            checks: CheckSettings {
                on_save_only: flag("onlyUpdateDiagnosticsOnSave"),
                no_lint: flag("disableElmLSDiagnostics"),
                review: severity.map(|severity| ReviewSettings {
                    severity,
                    program: path("elmReviewPath"),
                    elm: path("elmPath"),
                    elm_format: path("elmFormatPath"),
                }),
            },
            skip_install_confirmation: flag("skipInstallPackageConfirmation"),
            extended_capabilities: ExtendedCapabilities {
                move_function: extended("moveFunctionRefactoringSupport"),
                expose_commands: extended("exposeUnexposeSupport"),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn settings_read_from_the_section_or_around_it() {
        let on = Settings {
            checks: CheckSettings {
                on_save_only: true,
                ..CheckSettings::default()
            },
            ..Settings::default()
        };
        let section = json!({ "onlyUpdateDiagnosticsOnSave": true });
        assert_eq!(Settings::read(&section), Some(on.clone()));
        assert_eq!(Settings::read(&json!({ "elmLS": section })), Some(on));
        assert_eq!(Settings::read(&json!({})), Some(Settings::default()));
        assert_eq!(Settings::read(&Value::Null), None);
    }
}
