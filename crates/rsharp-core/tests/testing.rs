use rsharp_core::testing::{TestAttachment, TestItem, TestState};
use std::path::PathBuf;

#[test]
fn test_identity_keeps_projects_rows_and_frameworks_distinct() {
    let mut test = TestItem {
        project: PathBuf::from("/tests/Tests.csproj"),
        fully_qualified_name: "Space.Tests.Double(1)".into(),
        display_name: "Space.Tests.Double(value: 1)".into(),
        ..Default::default()
    };
    assert_eq!(test.method(), "Space.Tests.Double");
    assert_eq!(test.id(), "/tests/Tests.csproj::Space.Tests.Double");
    test.row = TestItem::row_of(&test.fully_qualified_name, &test.display_name);
    assert_eq!(test.row.as_deref(), Some("Double(value: 1)"));
    test.framework = Some("net10.0".into());
    assert_eq!(test.key(), "net10.0|/tests/Tests.csproj::Space.Tests.Double::Double(value: 1)");
    assert_eq!(TestItem::row_of("Space.Tests.Pass", "A friendly name"), None);
    assert_eq!(TestItem::row_of("Space.Tests.Pass", "Space.Tests.Pass"), None);
}

#[test]
fn result_attachments_decode_file_uris_without_reinterpreting_plain_paths() {
    let attached = TestAttachment::of("", "file:///tmp/a%20b%23.png");
    assert_eq!(attached.path, PathBuf::from("/tmp/a b#.png"));
    assert_eq!(attached.name, "a b#.png");
    assert_eq!(TestAttachment::of("frame", "/tmp/a%20b.png").path, PathBuf::from("/tmp/a%20b.png"));
}

#[test]
fn outcome_filters_can_persist_their_states() {
    assert_eq!(
        serde_json::from_str::<TestState>(&serde_json::to_string(&TestState::Failed).unwrap()).unwrap(),
        TestState::Failed
    );
}
