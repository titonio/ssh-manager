//! The frame seam's filtering, re-pointed from the deleted inline picker.
//!
//! The parent spec (#31) requires that "filtering (`compute_matches`) …
//! remain pure and move into the frame seam". These tests are the picker's
//! own filtering coverage, unchanged in behaviour, now asserting against
//! `sshm::frame` — the seam that owns them. The old picker's *render*
//! coverage died with the surface it gated; this did not, because the
//! filtering is still very much live.

use fuzzy_matcher::skim::SkimMatcherV2;
use sshm::config::Connection;
use sshm::frame::{build_row_text, compute_field_offsets, compute_matches};

fn make_conn(alias: &str, host: &str, user: &str, port: u16, folder: Option<&str>) -> Connection {
    Connection {
        id: String::new(),
        alias: alias.to_string(),
        host: host.to_string(),
        user: user.to_string(),
        port,
        key_path: None,
        folder: folder.map(|s| s.to_string()),
    }
}

// ── compute_matches ──────────────────────────────────────────────────────────

#[test]
fn empty_query_returns_all() {
    let conns = vec![
        make_conn("alpha", "h1", "u1", 22, None),
        make_conn("beta", "h2", "u2", 22, None),
    ];
    let matcher = SkimMatcherV2::default();
    let results = compute_matches(&conns, &matcher, "");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].1, 0);
    assert!(results[0].2.is_empty());
}

#[test]
fn folder_is_searched() {
    let conns = vec![
        make_conn("web", "web.com", "www", 22, Some("production")),
        make_conn("db", "db.com", "dba", 5432, Some("staging")),
        make_conn("app", "app.com", "dev", 22, None),
    ];
    let matcher = SkimMatcherV2::default();
    let results = compute_matches(&conns, &matcher, "prod");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 0); // original index of "web"
}

#[test]
fn folder_staging_matches_only_its_own_connection() {
    let conns = vec![
        make_conn("web", "web.com", "www", 22, Some("production")),
        make_conn("db", "db.com", "dba", 5432, Some("staging")),
    ];
    let matcher = SkimMatcherV2::default();
    let results = compute_matches(&conns, &matcher, "stag");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 1);
}

#[test]
fn results_are_ordered_by_score() {
    let conns = vec![
        make_conn("xyz-server", "host-a", "u", 22, None),
        make_conn("abc-server", "host-b", "u", 22, None),
        make_conn("server-abc", "host-c", "u", 22, None),
    ];
    let matcher = SkimMatcherV2::default();
    let results = compute_matches(&conns, &matcher, "abc");
    assert!(results.len() >= 2);
    assert!(results[0].1 >= results[1].1);
}

#[test]
fn no_match_returns_empty() {
    let conns = vec![
        make_conn("alpha", "h1", "u1", 22, None),
        make_conn("beta", "h2", "u2", 22, None),
    ];
    let matcher = SkimMatcherV2::default();
    let results = compute_matches(&conns, &matcher, "zzzznonexistent");
    assert!(results.is_empty());
}

#[test]
fn highlights_are_returned_for_a_match() {
    let conns = vec![make_conn("server", "host.com", "u", 22, None)];
    let matcher = SkimMatcherV2::default();
    let results = compute_matches(&conns, &matcher, "ser");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].2, vec![0, 1, 2]);
}

#[test]
fn user_field_is_searched() {
    let conns = vec![
        make_conn("web", "web.com", "admin", 22, None),
        make_conn("db", "db.com", "www", 22, None),
    ];
    let matcher = SkimMatcherV2::default();
    let results = compute_matches(&conns, &matcher, "adm");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 0);
}

#[test]
fn host_field_is_searched() {
    let conns = vec![
        make_conn("web", "web.com", "u", 22, None),
        make_conn("db", "database.internal", "u", 22, None),
    ];
    let matcher = SkimMatcherV2::default();
    let results = compute_matches(&conns, &matcher, "database");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, 1);
}

// ── build_row_text ───────────────────────────────────────────────────────────

#[test]
fn row_text_without_folder() {
    let conn = make_conn("web", "example.com", "www", 22, None);
    assert_eq!(build_row_text(&conn), "web (www@example.com:22)");
}

#[test]
fn row_text_with_folder() {
    let conn = make_conn(
        "prod-web",
        "prod.example.com",
        "admin",
        22,
        Some("production"),
    );
    assert_eq!(
        build_row_text(&conn),
        "[production] prod-web (admin@prod.example.com:22)"
    );
}

#[test]
fn row_text_with_non_default_port() {
    let conn = make_conn("dev", "localhost", "dev", 2222, None);
    assert_eq!(build_row_text(&conn), "dev (dev@localhost:2222)");
}

// ── compute_field_offsets ────────────────────────────────────────────────────

#[test]
fn field_offsets_without_folder() {
    let conn = make_conn("web", "example.com", "www", 22, None);
    let offsets = compute_field_offsets(&conn);
    // alias starts at 0, ends at 3
    assert_eq!(offsets[0], ("alias", 0, 3));
    // user starts at 5 (alias(3) + " (" (2)), ends at 8
    assert_eq!(offsets[1], ("user", 5, 8));
    // host starts at 9 (user(8) + "@" (1))
    assert_eq!(offsets[2].0, "host");
}

#[test]
fn field_offsets_with_folder() {
    let conn = make_conn("web", "example.com", "www", 22, Some("prod"));
    let offsets = compute_field_offsets(&conn);
    // folder: positions 1..5 (after `[`, before `]`)
    assert_eq!(offsets[0], ("folder", 1, 5));
    // alias starts at 7 (folder.len(4) + 3 = `[` `]` ` ` + 1)
    assert_eq!(offsets[1], ("alias", 7, 10));
}

#[test]
fn field_offsets_stay_in_sync_with_row_text() {
    // compute_field_offsets and build_row_text must agree on row layout.
    // If build_row_text changes, this test catches drift.
    let cases = vec![
        make_conn("web", "example.com", "www", 22, None),
        make_conn(
            "prod-web",
            "prod.example.com",
            "admin",
            22,
            Some("production"),
        ),
        make_conn("dev", "localhost", "dev", 2222, Some("staging")),
    ];
    for conn in cases {
        let text = build_row_text(&conn);
        let offsets = compute_field_offsets(&conn);

        // Field ranges must not overlap.
        for (i, (_, si, ei)) in offsets.iter().enumerate() {
            for (_, sj, ej) in offsets.iter().skip(i + 1) {
                assert!(
                    ei <= sj || ej <= si,
                    "overlap: ({},{}) vs ({},{})",
                    si,
                    ei,
                    sj,
                    ej
                );
            }
        }

        // Each field's offset must be within text bounds.
        for (_, s, e) in &offsets {
            assert!(*s < text.len() && *e <= text.len());
        }

        // Start positions must be monotonically increasing.
        for windows in offsets.windows(2) {
            assert!(
                windows[0].1 < windows[1].1,
                "field order drift: {:?} should come before {:?}",
                windows[0].0,
                windows[1].0
            );
        }

        // Each field's substring in the display text must match the actual value.
        for (name, s, e) in &offsets {
            let display_field = &text[*s..*e];
            let actual: &str = match *name {
                "folder" => conn.folder.as_deref().unwrap_or(""),
                "alias" => &conn.alias,
                "user" => &conn.user,
                "host" => &conn.host,
                "port" => &conn.port.to_string(),
                _ => panic!("unknown field: {}", name),
            };
            assert_eq!(
                display_field, actual,
                "field {:?} text drift: display={:?}, actual={:?}",
                name, display_field, actual
            );
        }
    }
}

// ── compute_matches maps indices to display positions ────────────────────────

#[test]
fn display_indices_are_mapped_to_the_row_text() {
    let conns = vec![make_conn("web", "web.com", "www", 22, Some("production"))];
    let matcher = SkimMatcherV2::default();
    let results = compute_matches(&conns, &matcher, "prod");
    assert_eq!(results.len(), 1);
    // "prod" matches positions 0..3 in "production".
    // Display: "[production] web (www@web.com:22)" → p,r,o,d at 1,2,3,4.
    assert_eq!(results[0].2, vec![1, 2, 3, 4]);
}

#[test]
fn alias_indices_have_zero_offset_without_a_folder() {
    // When there's no folder, alias starts at 0, so display = field positions.
    let conns = vec![make_conn("server", "host.com", "u", 22, None)];
    let matcher = SkimMatcherV2::default();
    let results = compute_matches(&conns, &matcher, "ser");
    assert_eq!(results[0].2, vec![0, 1, 2]);
}
