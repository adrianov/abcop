//! Host-path recognition for HTML and template files that may embed JS.

/// True when `path` is an HTML/template host that may embed JS.
pub(crate) fn is_host(path: &std::path::Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    is_whole_js_file(&lower) || HOST_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

pub(super) fn is_whole_js_file(name: &str) -> bool {
    WHOLE_JS_SUFFIXES.iter().any(|s| name.ends_with(s))
}

pub(super) fn is_module_host(name: &str) -> bool {
    name.ends_with(".vue") || name.ends_with(".svelte")
}

const WHOLE_JS_SUFFIXES: &[&str] = &[
    ".js.erb",
    ".mjs.erb",
    ".cjs.erb",
    ".jsx.erb",
    ".ts.erb",
    ".tsx.erb",
    ".mts.erb",
    ".cts.erb",
    ".js.j2",
    ".ts.j2",
    ".js.jinja",
    ".ts.jinja",
    ".js.jinja2",
    ".ts.jinja2",
    ".js.liquid",
    ".ts.liquid",
    ".js.twig",
    ".ts.twig",
    ".js.ejs",
    ".ts.ejs",
];

const HOST_SUFFIXES: &[&str] = &[
    ".html",
    ".htm",
    ".xhtml",
    ".html.erb",
    ".htm.erb",
    ".html.slim",
    ".html.haml",
    ".erb",
    ".slim",
    ".haml",
    ".pug",
    ".jade",
    ".vue",
    ".svelte",
    ".ejs",
    ".njk",
    ".nunjucks",
    ".jinja",
    ".jinja2",
    ".j2",
    ".djhtml",
    ".twig",
    ".liquid",
    ".hbs",
    ".handlebars",
    ".mustache",
    ".rhtml",
];
