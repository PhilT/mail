use mail::compose::{Attachments, Choice};
use std::path::PathBuf;

#[test]
fn apply_without_attachments_leaves_draft_unchanged() {
    let draft = "To: a@b.com\nSubject: hi\n\nHello\n";
    assert_eq!(Attachments::default().apply(draft), draft);
}

#[test]
fn apply_appends_an_mml_part_per_file() {
    let mut atts = Attachments::default();
    atts.add(PathBuf::from("/data/downloads/my invoice.pdf")).unwrap();
    atts.add(PathBuf::from("/data/pictures/photo.jpg")).unwrap();
    assert_eq!(
        atts.apply("To: a@b.com\n\nHello\n"),
        "To: a@b.com\n\nHello\n\
         <#part filename=\"/data/downloads/my invoice.pdf\"><#/part>\n\
         <#part filename=\"/data/pictures/photo.jpg\"><#/part>\n"
    );
}

#[test]
fn apply_starts_parts_on_their_own_line() {
    let mut atts = Attachments::default();
    atts.add(PathBuf::from("/data/a.pdf")).unwrap();
    assert_eq!(
        atts.apply("To: a@b.com\n\nHello"),
        "To: a@b.com\n\nHello\n<#part filename=\"/data/a.pdf\"><#/part>\n"
    );
}

#[test]
fn add_ignores_a_file_already_attached() {
    let mut atts = Attachments::default();
    atts.add(PathBuf::from("/data/a.pdf")).unwrap();
    atts.add(PathBuf::from("/data/a.pdf")).unwrap();
    assert_eq!(atts.names(), vec!["a.pdf"]);
}

#[test]
fn add_rejects_paths_mml_cannot_quote() {
    let mut atts = Attachments::default();
    assert!(atts.add(PathBuf::from("/data/say \"hi\".txt")).is_err());
    assert!(atts.add(PathBuf::from("/data/back\\slash.txt")).is_err());
    assert!(atts.names().is_empty());
}

#[test]
fn menu_keys_map_to_choices() {
    assert_eq!(Choice::from_key('s'), Some(Choice::Send));
    assert_eq!(Choice::from_key('a'), Some(Choice::Attach));
    assert_eq!(Choice::from_key('e'), Some(Choice::Edit));
    assert_eq!(Choice::from_key('d'), Some(Choice::Discard));
    assert_eq!(Choice::from_key('x'), None);
}
