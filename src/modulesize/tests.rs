//! Classification behavior for route tables and third-party trees.

use super::classify::{is_route_table, is_third_party};

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
