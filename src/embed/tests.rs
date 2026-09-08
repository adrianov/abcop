//! Vectors: embedded JS in HTML/templates is scored; markup alone is quiet.

use crate::abc::Limits;
use crate::modulesize;
use crate::pipeline::analyze_src;

fn limits() -> Limits {
    Limits {
        method: 17.0,
        module: modulesize::MAX_ABC,
    }
}

fn never_names(path: &str, src: &str) -> Vec<String> {
    analyze_src(std::path::Path::new(path), src.as_bytes(), None, limits())
        .never_used
        .into_iter()
        .map(|o| o.name)
        .collect()
}

fn used_names(path: &str, src: &str) -> Vec<String> {
    analyze_src(std::path::Path::new(path), src.as_bytes(), None, limits())
        .used_once
        .into_iter()
        .map(|o| o.name)
        .collect()
}

#[test]
fn html_script_never_used_and_line_remap() {
    let r = analyze_src(
        std::path::Path::new("page.html"),
        b"<!doctype html>\n<html><body>\n<script>\nconst unused = 1;\n</script>\n</body></html>\n",
        None,
        limits(),
    );
    assert_eq!(
        r.never_used
            .iter()
            .map(|o| (o.name.as_str(), o.line))
            .collect::<Vec<_>>(),
        vec![("unused", 4)]
    );
}

#[test]
fn html_without_script_stays_clean() {
    let r = analyze_src(
        std::path::Path::new("index.html"),
        b"<html lang=\"en\"><body><p>Hi</p></body></html>\n",
        None,
        limits(),
    );
    assert!(r.is_clean());
}

#[test]
fn html_skips_json_ld_script() {
    let src = concat!(
        "<html><script type=\"application/ld+json\">\n",
        "{\"unused\": 1}\n",
        "</script></html>\n"
    );
    assert!(
        analyze_src(std::path::Path::new("seo.html"), src.as_bytes(), None, limits()).is_clean()
    );
}

#[test]
fn erb_script_with_ruby_holes() {
    let src = "<script>\nconst unused = <%= @n %>;\nconst sum = 1 + 2;\nconsole.log(sum);\n</script>\n";
    assert_eq!(never_names("show.html.erb", src), vec!["unused"]);
}

#[test]
fn slim_javascript_filter() {
    let src = "div\n  javascript:\n    const unused = 1\n  p hello\n";
    assert_eq!(never_names("show.slim", src), vec!["unused"]);
}

#[test]
fn slim_javascript_one_liner() {
    assert_eq!(
        never_names("show.slim", "div\n  javascript: const unused = 1\n  p hi\n"),
        vec!["unused"]
    );
}

#[test]
fn html_script_abc_remaps_to_host_line() {
    let src = concat!(
        "<html><body>\n",
        "<script>\n",
        "function checkout(user, items) {\n",
        "  let total = 0;\n",
        "  for (const it of items) {\n",
        "    total = total + it.price;\n",
        "  }\n",
        "  if (user.vip) {\n",
        "    total = applyDiscount(total);\n",
        "  }\n",
        "  log(total);\n",
        "}\n",
        "</script>\n",
        "</body></html>\n"
    );
    let o = analyze_src(
        std::path::Path::new("pay.html"),
        src.as_bytes(),
        None,
        Limits {
            method: 0.0,
            module: modulesize::MAX_ABC,
        },
    )
    .abc
    .into_iter()
    .find(|o| o.name == "checkout")
    .expect("abc");
    assert_eq!(o.line, 3);
    assert_eq!(o.vector, "<3, 3, 2>");
}

#[test]
fn jinja_script_block() {
    let src = "<script>\nconst unused = {{ n }};\n</script>\n";
    assert_eq!(never_names("page.jinja", src), vec!["unused"]);
}

#[test]
fn vue_sfc_script_function_local() {
    let src = "<template><p/></template>\n<script>\nfunction boot() {\n  const unused = 1;\n}\n</script>\n";
    assert_eq!(never_names("App.vue", src), vec!["unused"]);
}

#[test]
fn js_erb_whole_file() {
    let src = "function boot() {\n  const unused = <%= value %>;\n  const once = 1;\n  console.log(once);\n}\n";
    assert_eq!(never_names("app.js.erb", src), vec!["unused"]);
    assert_eq!(used_names("app.js.erb", src), vec!["once"]);
}

#[test]
fn host_paths_are_code() {
    for p in [
        "a.html",
        "a.htm",
        "a.html.erb",
        "a.erb",
        "a.slim",
        "a.haml",
        "a.vue",
        "a.svelte",
        "a.jinja",
        "a.j2",
        "a.twig",
        "a.ejs",
        "a.js.erb",
    ] {
        assert!(
            crate::paths::is_code_path(std::path::Path::new(p)),
            "{p} should be a code path"
        );
        assert!(
            crate::embed::is_host(std::path::Path::new(p)),
            "{p} should be a JS host"
        );
    }
}
