//! Classification and ModuleAbcSize scoring.

use super::classify::{is_fixture_tree, is_route_table, is_third_party};
use super::{ModuleAbc, finding_message, measure};
use crate::abc::AbcOffense;

#[test]
fn rails_route_tables_are_route_files() {
    for p in [
        "config/routes.rb",
        "config/routes/api.rb",
        "engines/billing/config/routes.rb",
        "engines/billing/config/routes/admin.rb",
    ] {
        assert!(is_route_table(std::path::Path::new(p)), "{p}");
    }
}

#[test]
fn ordinary_sources_are_not_route_files() {
    for p in [
        "app/models/route.rb",
        "config/application.rb",
        "config/routes_helper_spec.rb.rb",
        "routes.md",
        "app/routes_loader.rb",
        "main.rb",
    ] {
        assert!(!is_route_table(std::path::Path::new(p)), "{p}");
    }
}

#[test]
fn third_party_trees_are_dropped_from_scope() {
    for p in [
        "vendor/tree-sitter-swift/src/scanner.c",
        "app/assets/node_modules/left-pad/index.js",
        "target/debug/foo.rs",
        "db/migrate/20260101120000_add_users.rb",
        "app/assets/builds/app.min.js",
        "lib/user_pb.rb",
        "/repo/vendor/x.go",
    ] {
        assert!(is_third_party(std::path::Path::new(p)), "{p}");
    }
}

#[test]
fn owned_sources_stay_in_scope() {
    for p in [
        "src/main.rs",
        "spec/models/user_spec.rb",
        "test/lib/format_test.rb",
        "app/models/user.rb",
        "vendor_all/owned.rb",
        "lib/pb.rb",
    ] {
        assert!(!is_third_party(std::path::Path::new(p)), "{p}");
        assert!(!is_fixture_tree(std::path::Path::new(p)), "{p}");
    }
}

#[test]
fn fixture_trees_are_dropped_from_scope() {
    for p in [
        "tests/fixtures/cops/style/global_vars/no_offense.rb",
        "fixtures/probe.rb",
        "spec/fixtures/users.yml.rb",
        "/repo/tests/fixtures/x.go",
    ] {
        assert!(is_fixture_tree(std::path::Path::new(p)), "{p}");
    }
}

fn offense(a: u32, b: u32, c: u32) -> AbcOffense {
    AbcOffense {
        line: 1,
        end_line: 1,
        column: 0,
        name: "m".into(),
        score: (((a * a + b * b + c * c) as f64).sqrt() * 100.0).round() / 100.0,
        vector: format!("<{a}, {b}, {c}>"),
        start_byte: 0,
        end_byte: 0,
        prior: None,
    }
}

#[test]
fn module_abc_sums_method_vectors() {
    let scores = vec![offense(30, 40, 0), offense(0, 0, 120)];
    let hit = measure(&scores, "app/models/user.rb", "", super::MAX_ABC).1.unwrap();
    assert_eq!(
        hit,
        ModuleAbc {
            score: 130.0,
            vector: "<30, 40, 120>".into(),
            prior: None,
        }
    );
}

#[test]
fn module_abc_ignores_scores_at_the_threshold() {
    // magnitude 120 exactly must not fire (AbcSize-style `>`).
    let scores = vec![offense(72, 96, 0)]; // 120
    assert!(measure(&scores, "app/models/user.rb", "", super::MAX_ABC).1.is_none());
}

#[test]
fn module_abc_respects_custom_max() {
    let scores = vec![offense(30, 40, 0)]; // 50
    assert!(measure(&scores, "app/models/user.rb", "", 50.0).1.is_none());
    assert!(measure(&scores, "app/models/user.rb", "", 49.0).1.is_some());
}

#[test]
fn rust_cfg_test_tail_is_excluded_from_module_abc() {
    let src = "fn prod() {}\n#[cfg(test)]\nmod tests {\n  fn t() {}\n}\n";
    let scores = vec![
        AbcOffense {
            line: 1,
            end_line: 1,
            column: 0,
            name: "prod".into(),
            score: 0.0,
            vector: "<0, 0, 0>".into(),
            start_byte: 0,
            end_byte: 0,
            prior: None,
        },
        AbcOffense {
            line: 4,
            end_line: 4,
            column: 2,
            name: "t".into(),
            score: 100.0,
            vector: "<60, 80, 0>".into(),
            start_byte: 0,
            end_byte: 0,
            prior: None,
        },
    ];
    assert!(measure(&scores, "src/lib.rs", src, super::MAX_ABC).1.is_none());
}

#[test]
fn risen_module_message_names_the_head_score() {
    let hit = ModuleAbc {
        score: 130.0,
        vector: "<130, 0, 0>".into(),
        prior: Some(121.0),
    };
    assert_eq!(
        finding_message(&hit, Some(120.0)),
        "Assignment Branch Condition size for module rose from 121 to 130 -- back to 121 or below. [<130, 0, 0> 130/120]"
    );
    assert_eq!(
        finding_message(&hit, None),
        "Assignment Branch Condition size for module rose from 121 to 130 -- back to 121 or below. [<130, 0, 0> 130]"
    );
}
