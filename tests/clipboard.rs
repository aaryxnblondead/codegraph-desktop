//! Clipboard tests: the copy buttons must really emit `CopyText` commands with
//! the analysis text, not just look like buttons.

use codegraph_desktop::report::{cycle_text, impact_text, warning_text, warnings_text};
use codegraph_desktop::structure::CutEdge;
use codegraph_desktop::ui::copy_button;
use codegraph_desktop::warnings::{Severity, Warning};
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

/// Click `label`, then step until egui emits the clipboard command.
fn click_and_capture(harness: &mut Harness<'_>, label: &str) -> Option<String> {
    harness.get_by_label(label).click();
    for _ in 0..4 {
        harness.step();
        if let Some(text) = harness
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
        {
            return Some(text);
        }
    }
    None
}

#[test]
fn copy_button_puts_text_on_the_clipboard() {
    let mut harness = Harness::new_ui(|ui| {
        copy_button(ui, "hello clipboard", "copy test");
    });
    harness.run();
    assert_eq!(
        click_and_capture(&mut harness, "Copy copy test").as_deref(),
        Some("hello clipboard"),
        "the button must emit CopyText with the payload"
    );
}

#[test]
fn warning_text_payload_is_the_analysis_text() {
    let warning = Warning {
        id: "w1".into(),
        severity: Severity::Error,
        rule: "cycle".into(),
        message: "2 files form a dependency cycle".into(),
        subjects: vec!["a.ts".into(), "b.ts".into()],
        count: 2,
        metrics: Default::default(),
        explanation: "loop".into(),
        suggestion: "Cut a.ts -> b.ts.".into(),
        locations: vec![codegraph_desktop::warnings::Location::file("a.ts")],
    };
    let payload = warning_text(&warning);

    let mut harness = Harness::new_ui(|ui| {
        copy_button(ui, &warning_text(&warning), "copy warning");
    });
    harness.run();

    let copied = click_and_capture(&mut harness, "Copy copy warning").expect("copy command");
    assert_eq!(copied, payload);
    assert!(copied.contains("[ERROR] cycle"));
    assert!(copied.contains("Cut a.ts -> b.ts."));
}

#[test]
fn bulk_payloads_are_copyable() {
    let cycle = cycle_text(
        &["a.ts".to_string(), "b.ts".to_string()],
        &[CutEdge {
            from: "b.ts".into(),
            to: "a.ts".into(),
            weight: 1,
        }],
    );
    let impact = impact_text("parse", &[(1, 2)], &[(1, 3)]);
    let warnings = warnings_text(&[]);
    for (payload, label) in [(cycle, "cycle"), (impact, "impact"), (warnings, "warnings")] {
        let expected = payload.clone();
        let mut harness = Harness::new_ui(move |ui| {
            copy_button(ui, &payload, "payload");
        });
        harness.run();
        assert_eq!(
            click_and_capture(&mut harness, "Copy payload").as_deref(),
            Some(expected.as_str()),
            "copy {label}"
        );
    }
}
