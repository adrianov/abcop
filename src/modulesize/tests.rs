//! Classification behavior for route tables and third-party trees.

use super::classify::{is_route_table, is_third_party};
use super::{ModuleAbc, SizeGate, from_scores};
use crate::abc::AbcOffense;

#[test]
fn size_gate_covers_matches_scope() {
    assert!(!SizeGate::Never.covers("app/models/x.rb"));
    assert!(!SizeGate::Never.covers("spec/models/x_spec.rb"));
    assert!(SizeGate::Both.covers("app/models/x.rb"));
    assert!(SizeGate::Both.covers("spec/models/x_spec.rb"));
    assert!(!SizeGate::Specs.covers("app/models/x.rb"));
    assert!(SizeGate::Specs.covers("spec/models/x_spec.rb"));
    assert!(SizeGate::Specs.covers("test/foo_test.rb"));
}

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
    }
}

fn offense(a: u32, b: u32, c: u32) -> AbcOffense {
    let raw = ((a * a + b * b + c * c) as f64).sqrt();
    AbcOffense {
        line: 1,
        end_line: 1,
        column: 0,
        name: "m".into(),
        score: (raw * 100.0).round() / 100.0,
        vector: format!("<{a}, {b}, {c}>"),
    }
}

#[test]
fn module_abc_sums_method_vectors() {
    let scores = vec![offense(30, 40, 0), offense(0, 0, 120)];
    let hit = from_scores(&scores, "app/models/user.rb", "", super::MAX_ABC).unwrap();
    assert_eq!(
        hit,
        ModuleAbc {
            score: 130.0,
            vector: "<30, 40, 120>".into(),
        }
    );
}

#[test]
fn module_abc_ignores_scores_at_the_threshold() {
    // magnitude 120 exactly must not fire (AbcSize-style `>`).
    let scores = vec![offense(72, 96, 0)]; // 120
    assert!(from_scores(&scores, "app/models/user.rb", "", super::MAX_ABC).is_none());
}

#[test]
fn module_abc_respects_custom_max() {
    let scores = vec![offense(30, 40, 0)]; // 50
    assert!(from_scores(&scores, "app/models/user.rb", "", 50.0).is_none());
    assert!(from_scores(&scores, "app/models/user.rb", "", 49.0).is_some());
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
        },
        AbcOffense {
            line: 4,
            end_line: 4,
            column: 2,
            name: "t".into(),
            score: 100.0,
            vector: "<60, 80, 0>".into(),
        },
    ];
    assert!(from_scores(&scores, "src/lib.rs", src, super::MAX_ABC).is_none());
}
