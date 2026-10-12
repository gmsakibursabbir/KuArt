//! Emitters: code IR → source text.

use std::collections::BTreeMap;

use crate::{El, tailwind};

const RESET: &str = "\
*, *::before, *::after { box-sizing: border-box; margin: 0; }
button { border: 0; background: none; padding: 0; font: inherit; color: inherit; cursor: pointer; }
hr { border: 0; }
img, svg { display: block; }
";

pub fn css(root: &El) -> String {
    let mut out = String::from(RESET);
    rules(root, &mut out);
    out
}

fn rules(el: &El, out: &mut String) {
    if !el.class.is_empty() && !el.decls.is_empty() {
        out.push_str(&format!("\n.{} {{\n", el.class));
        for (k, v) in &el.decls {
            out.push_str(&format!("  {k}: {v};\n"));
        }
        out.push_str("}\n");
    }
    for c in &el.children {
        rules(c, out);
    }
}

pub fn html(root: &El) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n<style>\n{}</style>\n</head>\n<body>\n{}</body>\n</html>\n",
        root.class,
        css(root),
        markup(root, Flavor::Html, 0)
    )
}

pub fn tailwind(root: &El) -> String {
    format!(
        "<!-- Tailwind CSS v3+ (arbitrary values where the design is off-scale) -->\n{}",
        markup(root, Flavor::Tailwind, 0)
    )
}

pub fn react(root: &El, component: &str) -> String {
    let body = markup(root, Flavor::Jsx, 2);
    format!(
        "import \"./{component}.css\";\n\nexport default function {component}() {{\n  return (\n{body}  );\n}}\n\n\
         /* ---- {component}.css ---- */\n{}",
        css(root)
    )
}

pub fn vue(root: &El) -> String {
    format!(
        "<template>\n{}</template>\n\n<style scoped>\n{}</style>\n",
        markup(root, Flavor::Html, 1),
        css(root)
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Flavor {
    Html,
    Jsx,
    Tailwind,
}

fn markup(el: &El, f: Flavor, depth: usize) -> String {
    let mut out = String::new();
    write_el(el, f, depth, &mut out);
    out
}

fn write_el(el: &El, f: Flavor, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    let class = match f {
        Flavor::Tailwind => tailwind::classes(&el.decls),
        _ => el.class.clone(),
    };
    let mut open = format!("<{}", el.tag);
    if !class.is_empty() {
        let attr = if f == Flavor::Jsx {
            "className"
        } else {
            "class"
        };
        open.push_str(&format!(" {attr}=\"{}\"", escape_attr(&class)));
    }
    for (k, v) in &el.attrs {
        open.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
    }
    if el.tag == "button" {
        open.push_str(" type=\"button\"");
    }
    if matches!(el.tag, "img" | "hr") {
        out.push_str(&format!(
            "{pad}{open}{}>\n",
            if f == Flavor::Jsx { " /" } else { "" }
        ));
        return;
    }
    let inline = el.children.is_empty() && el.raw.is_none();
    if inline {
        let text = el
            .text
            .as_deref()
            .map_or(String::new(), |t| escape_text(t, f));
        out.push_str(&format!("{pad}{open}>{text}</{}>\n", el.tag));
        return;
    }
    out.push_str(&format!("{pad}{open}>\n"));
    if let Some(raw) = &el.raw {
        let svg = if f == Flavor::Jsx {
            jsx_svg(raw)
        } else {
            raw.clone()
        };
        out.push_str(&format!("{pad}  {svg}\n"));
    }
    if let Some(t) = &el.text
        && el.children.is_empty()
    {
        out.push_str(&format!("{pad}  {}\n", escape_text(t, f)));
    }
    let spans = el
        .children
        .iter()
        .all(|c| c.tag == "span" && c.children.is_empty());
    if spans && el.text.is_none() && !el.children.is_empty() {
        // Rich text stays on one line so no whitespace is added between runs.
        out.push_str(&pad);
        out.push_str("  ");
        for c in &el.children {
            let mut s = String::new();
            write_el(c, f, 0, &mut s);
            out.push_str(s.trim_end());
        }
        out.push('\n');
    } else {
        for c in &el.children {
            write_el(c, f, depth + 1, out);
        }
    }
    out.push_str(&format!("{pad}</{}>\n", el.tag));
}

fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

fn escape_text(s: &str, f: Flavor) -> String {
    let s = s
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let s = if f == Flavor::Jsx {
        s.replace('{', "{'{'}").replace('}', "{'}'}")
    } else {
        s
    };
    let br = if f == Flavor::Jsx { "<br />" } else { "<br>" };
    s.trim_end_matches('\n')
        .replace("\r\n", "\n")
        .replace('\n', br)
}

/// SVG markup → JSX: hyphenated and namespaced attributes become camelCase.
pub(crate) fn jsx_svg(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());
    let mut in_tag = false;
    let mut in_value = false;
    let mut chars = svg.chars().peekable();
    let mut name = String::new();
    while let Some(c) = chars.next() {
        if in_value {
            out.push(c);
            if c == '"' {
                in_value = false;
            }
            continue;
        }
        match c {
            '<' => {
                in_tag = true;
                out.push(c);
                // Element name is copied as is.
                while let Some(&n) = chars.peek() {
                    if n.is_whitespace() || n == '>' || n == '/' {
                        break;
                    }
                    out.push(n);
                    chars.next();
                }
            }
            '>' => {
                in_tag = false;
                out.push(c);
            }
            '"' if in_tag => {
                in_value = true;
                out.push(c);
            }
            c if in_tag && (c.is_ascii_alphanumeric() || c == '-' || c == ':') => {
                name.push(c);
                if chars
                    .peek()
                    .is_some_and(|&n| !(n.is_ascii_alphanumeric() || n == '-' || n == ':'))
                {
                    out.push_str(&camel(&std::mem::take(&mut name)));
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn camel(attr: &str) -> String {
    match attr {
        "class" => return "className".into(),
        "xmlns:xlink" => return "xmlnsXlink".into(),
        "xlink:href" => return "xlinkHref".into(),
        _ => {}
    }
    if attr.starts_with("data-") || attr.starts_with("aria-") {
        return attr.into();
    }
    let mut out = String::new();
    let mut up = false;
    for c in attr.chars() {
        if c == '-' || c == ':' {
            up = true;
        } else if up {
            out.push(c.to_ascii_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Design tokens: colours, font sizes, families, radii and spacing used in the design.
pub fn tokens(root: &El) -> String {
    let mut groups: BTreeMap<&str, BTreeMap<String, usize>> = BTreeMap::new();
    collect(root, &mut groups);
    let mut css = String::from(":root {\n");
    let mut json = String::from("{\n");
    let order = [
        "color",
        "font-family",
        "font-size",
        "font-weight",
        "radius",
        "spacing",
    ];
    let mut first_group = true;
    for g in order {
        let Some(values) = groups.get(g) else {
            continue;
        };
        // Most used first; ties by value for determinism.
        let mut v: Vec<(&String, &usize)> = values.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        if !first_group {
            json.push_str(",\n");
        }
        first_group = false;
        let kind = match g {
            "color" => "color",
            "font-family" => "fontFamily",
            "font-weight" => "fontWeight",
            _ => "dimension",
        };
        json.push_str(&format!("  \"{g}\": {{\n"));
        for (i, (value, _)) in v.iter().enumerate() {
            css.push_str(&format!("  --{g}-{}: {value};\n", i + 1));
            let sep = if i + 1 < v.len() { "," } else { "" };
            json.push_str(&format!(
                "    \"{}\": {{ \"$type\": \"{kind}\", \"$value\": \"{}\" }}{sep}\n",
                i + 1,
                value.replace('"', "\\\"")
            ));
        }
        json.push_str("  }");
    }
    css.push_str("}\n");
    json.push_str("\n}\n");
    format!("/* tokens.css */\n{css}\n/* tokens.json (W3C design tokens format) */\n{json}")
}

fn collect(el: &El, groups: &mut BTreeMap<&str, BTreeMap<String, usize>>) {
    for (k, v) in &el.decls {
        let group = match *k {
            "color" | "background" | "border" | "outline" | "border-top" | "box-shadow" => {
                for c in colors(v) {
                    *groups.entry("color").or_default().entry(c).or_default() += 1;
                }
                continue;
            }
            "font-family" => "font-family",
            "font-size" => "font-size",
            "font-weight" => "font-weight",
            "border-radius" if v != "50%" => "radius",
            "gap" | "padding" => {
                for part in v.split_whitespace() {
                    *groups
                        .entry("spacing")
                        .or_default()
                        .entry(part.to_owned())
                        .or_default() += 1;
                }
                continue;
            }
            _ => continue,
        };
        *groups
            .entry(group)
            .or_default()
            .entry(v.clone())
            .or_default() += 1;
    }
    for c in &el.children {
        collect(c, groups);
    }
}

/// `#rrggbb` and `rgba(...)` colours inside a CSS value.
fn colors(v: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = v;
    while let Some(i) = rest.find(['#', 'r']) {
        let tail = &rest[i..];
        if tail.starts_with('#')
            && tail.len() >= 7
            && tail[1..7].chars().all(|c| c.is_ascii_hexdigit())
        {
            out.push(tail[..7].to_owned());
            rest = &tail[7..];
        } else if tail.starts_with("rgba(")
            && let Some(end) = tail.find(')')
        {
            out.push(tail[..=end].to_owned());
            rest = &tail[end + 1..];
        } else {
            rest = &tail[1..];
        }
    }
    out
}
