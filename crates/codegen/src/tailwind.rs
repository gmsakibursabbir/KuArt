//! CSS declarations → Tailwind utilities. Values on the default scale use its keys; anything
//! else uses arbitrary values (`w-[13px]`), so the output always matches the design.

/// Tailwind classes for `decls`, in roughly the official order (layout → box → type → visual).
pub fn classes(decls: &[(&'static str, String)]) -> String {
    let mut out: Vec<String> = Vec::new();
    for (k, v) in decls {
        let v = v.as_str();
        match *k {
            "position" => out.push(v.into()),
            "left" => out.push(format!("left-{}", spacing(v))),
            "top" => out.push(format!("top-{}", spacing(v))),
            "display" => out.push(if v == "flex" { "flex".into() } else { v.into() }),
            "flex-direction" if v == "column" => out.push("flex-col".into()),
            "flex-wrap" if v == "wrap" => out.push("flex-wrap".into()),
            "flex" if v == "1 1 0" => out.push("flex-1".into()),
            "align-self" if v == "stretch" => out.push("self-stretch".into()),
            "justify-content" => out.push(
                match v {
                    "center" => "justify-center",
                    "flex-end" => "justify-end",
                    "space-between" => "justify-between",
                    _ => "justify-start",
                }
                .into(),
            ),
            "align-items" => out.push(
                match v {
                    "center" => "items-center",
                    "flex-end" => "items-end",
                    "baseline" => "items-baseline",
                    "stretch" => "items-stretch",
                    _ => "items-start",
                }
                .into(),
            ),
            "gap" => match v.split_once(' ') {
                Some((row, col)) => {
                    out.push(format!("gap-x-{}", spacing(col)));
                    out.push(format!("gap-y-{}", spacing(row)));
                }
                None => out.push(format!("gap-{}", spacing(v))),
            },
            "padding" => padding(v, &mut out),
            "margin-left" => out.push(format!("ml-{}", spacing(v))),
            "margin-top" => out.push(format!("mt-{}", spacing(v))),
            "width" => out.push(format!("w-{}", spacing(v))),
            "height" => out.push(format!("h-{}", spacing(v))),
            "overflow" if v == "hidden" => out.push("overflow-hidden".into()),
            "background" if v.starts_with('#') || v.starts_with("rgba(") => {
                out.push(format!("bg-[{}]", arb(v)));
            }
            "background" => out.push(format!("[background:{}]", arb(v))),
            "border" | "outline" => border(k, v, &mut out),
            "border-radius" => out.push(radius(v)),
            "color" => out.push(format!("text-[{}]", arb(v))),
            "font-family" => {
                let first = v.split(',').next().unwrap_or(v).trim().trim_matches('"');
                out.push(format!("font-['{}']", arb(first)));
            }
            "font-size" => out.push(format!("text-[{v}]")),
            "font-weight" => out.push(weight(v)),
            "font-style" if v == "italic" => out.push("italic".into()),
            "line-height" => out.push(format!("leading-[{v}]")),
            "letter-spacing" => out.push(format!("tracking-[{v}]")),
            "text-align" => out.push(format!("text-{v}")),
            "text-decoration" => out.push(
                if v == "underline" {
                    "underline"
                } else {
                    "line-through"
                }
                .into(),
            ),
            "text-transform" => out.push(match v {
                "uppercase" => "uppercase".into(),
                "lowercase" => "lowercase".into(),
                _ => "capitalize".into(),
            }),
            "white-space" if v == "nowrap" => out.push("whitespace-nowrap".into()),
            "opacity" => out.push(format!("opacity-[{v}]")),
            "box-shadow" => out.push(format!("shadow-[{}]", arb(v))),
            "filter" => out.push(format!("blur-[{}]", inner(v))),
            "backdrop-filter" => out.push(format!("backdrop-blur-[{}]", inner(v))),
            "transform" => out.push(format!("rotate-[{}]", inner(v))),
            "transform-origin" if v == "top left" => out.push("origin-top-left".into()),
            "object-fit" => out.push(format!("object-{v}")),
            _ => out.push(format!("[{k}:{}]", arb(v))),
        }
    }
    out.join(" ")
}

/// Spaces are written as underscores inside arbitrary values.
fn arb(v: &str) -> String {
    v.replace(' ', "_")
}

/// The argument of `name(arg)`.
fn inner(v: &str) -> &str {
    v.split_once('(')
        .and_then(|(_, r)| r.strip_suffix(')'))
        .unwrap_or(v)
}

fn px_value(v: &str) -> Option<f32> {
    if v == "0" {
        return Some(0.0);
    }
    v.strip_suffix("px")?.parse().ok()
}

/// Default spacing scale key (`4`, `0.5`, `px`) or an arbitrary value.
fn spacing(v: &str) -> String {
    if let Some(p) = px_value(v) {
        if p == 1.0 {
            return "px".into();
        }
        let q = p / 4.0;
        let on_scale = (q * 2.0).fract() == 0.0
            && (q <= 3.5
                || (q.fract() == 0.0
                    && (q <= 12.0
                        || [
                            14.0, 16.0, 20.0, 24.0, 28.0, 32.0, 36.0, 40.0, 44.0, 48.0, 52.0, 56.0,
                            60.0, 64.0, 72.0, 80.0, 96.0,
                        ]
                        .contains(&q))));
        if on_scale && p >= 0.0 {
            return crate::num(q);
        }
    }
    format!("[{}]", arb(v))
}

fn padding(v: &str, out: &mut Vec<String>) {
    let p: Vec<&str> = v.split_whitespace().collect();
    let [t, r, b, l] = match p.as_slice() {
        [a] => [*a; 4],
        [a, b] => [*a, *b, *a, *b],
        [a, b, c] => [*a, *b, *c, *b],
        [a, b, c, d] => [*a, *b, *c, *d],
        _ => return,
    };
    if t == r && r == b && b == l {
        out.push(format!("p-{}", spacing(t)));
    } else if t == b && r == l {
        out.push(format!("px-{}", spacing(r)));
        out.push(format!("py-{}", spacing(t)));
    } else {
        for (side, val) in [("pt", t), ("pr", r), ("pb", b), ("pl", l)] {
            if val != "0" {
                out.push(format!("{side}-{}", spacing(val)));
            }
        }
    }
}

fn border(prop: &str, v: &str, out: &mut Vec<String>) {
    let mut parts = v.splitn(3, ' ');
    let (width, style, color) = (
        parts.next().unwrap_or("1px"),
        parts.next().unwrap_or("solid"),
        parts.next().unwrap_or(""),
    );
    out.push(if width == "1px" {
        prop.to_owned()
    } else {
        format!("{prop}-[{width}]")
    });
    if style == "dashed" {
        out.push(format!("{prop}-dashed"));
    }
    if !color.is_empty() {
        out.push(format!("{prop}-[{}]", arb(color)));
    }
}

fn radius(v: &str) -> String {
    match v {
        "50%" => "rounded-full".into(),
        "2px" => "rounded-sm".into(),
        "4px" => "rounded".into(),
        "6px" => "rounded-md".into(),
        "8px" => "rounded-lg".into(),
        "12px" => "rounded-xl".into(),
        "16px" => "rounded-2xl".into(),
        "24px" => "rounded-3xl".into(),
        _ => format!("rounded-[{}]", arb(v)),
    }
}

fn weight(v: &str) -> String {
    match v {
        "100" => "font-thin",
        "200" => "font-extralight",
        "300" => "font-light",
        "400" => "font-normal",
        "500" => "font-medium",
        "600" => "font-semibold",
        "700" => "font-bold",
        "800" => "font-extrabold",
        "900" => "font-black",
        _ => return format!("font-[{v}]"),
    }
    .into()
}
