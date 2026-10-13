//! A small LaTeX math typesetter for the chat: fractions, roots, super- and
//! subscripts, `\text`, big operators and the usual symbols, laid out with
//! GPUI elements (no JS engine, no font shaping of our own). Inline `$…$`
//! math becomes a Unicode line instead (see [`inline`]).

use gpui::prelude::FluentBuilder as _;
use gpui::{AnyElement, Hsla, IntoElement, ParentElement as _, Styled as _, div, px};

#[derive(Debug, Clone, PartialEq)]
enum Node {
    /// A symbol or run of characters; `italic` for single variables.
    Sym { text: String, italic: bool },
    Text(String),
    Row(Vec<Node>),
    Scripts { base: Box<Node>, sub: Option<Box<Node>>, sup: Option<Box<Node>> },
    Frac(Box<Node>, Box<Node>),
    Sqrt(Box<Node>),
    /// `\\` inside a display block.
    Break,
}

fn symbol(cmd: &str) -> Option<&'static str> {
    Some(match cmd {
        "cdot" => "·",
        "times" => "×",
        "div" => "÷",
        "pm" => "±",
        "mp" => "∓",
        "infty" => "∞",
        "to" | "rightarrow" => "→",
        "leftarrow" | "gets" => "←",
        "Rightarrow" | "implies" => "⇒",
        "Leftarrow" => "⇐",
        "Leftrightarrow" | "iff" => "⇔",
        "leftrightarrow" => "↔",
        "mapsto" => "↦",
        "le" | "leq" => "≤",
        "ge" | "geq" => "≥",
        "ne" | "neq" => "≠",
        "approx" => "≈",
        "equiv" => "≡",
        "sim" => "∼",
        "propto" => "∝",
        "sum" => "∑",
        "prod" => "∏",
        "int" => "∫",
        "iint" => "∬",
        "oint" => "∮",
        "partial" => "∂",
        "nabla" => "∇",
        "forall" => "∀",
        "exists" => "∃",
        "in" => "∈",
        "notin" => "∉",
        "subset" => "⊂",
        "subseteq" => "⊆",
        "supset" => "⊃",
        "cup" => "∪",
        "cap" => "∩",
        "emptyset" | "varnothing" => "∅",
        "land" | "wedge" => "∧",
        "lor" | "vee" => "∨",
        "neg" | "lnot" => "¬",
        "circ" => "∘",
        "bullet" => "•",
        "star" => "⋆",
        "ldots" | "dots" | "cdots" => "…",
        "prime" => "′",
        "degree" => "°",
        "angle" => "∠",
        "perp" => "⊥",
        "parallel" => "∥",
        "langle" => "⟨",
        "rangle" => "⟩",
        "lfloor" => "⌊",
        "rfloor" => "⌋",
        "lceil" => "⌈",
        "rceil" => "⌉",
        "alpha" => "α",
        "beta" => "β",
        "gamma" => "γ",
        "delta" => "δ",
        "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ",
        "eta" => "η",
        "theta" | "vartheta" => "θ",
        "iota" => "ι",
        "kappa" => "κ",
        "lambda" => "λ",
        "mu" => "μ",
        "nu" => "ν",
        "xi" => "ξ",
        "pi" => "π",
        "rho" => "ρ",
        "sigma" => "σ",
        "tau" => "τ",
        "upsilon" => "υ",
        "phi" | "varphi" => "φ",
        "chi" => "χ",
        "psi" => "ψ",
        "omega" => "ω",
        "Gamma" => "Γ",
        "Delta" => "Δ",
        "Theta" => "Θ",
        "Lambda" => "Λ",
        "Xi" => "Ξ",
        "Pi" => "Π",
        "Sigma" => "Σ",
        "Phi" => "Φ",
        "Psi" => "Ψ",
        "Omega" => "Ω",
        "hbar" => "ℏ",
        "ell" => "ℓ",
        "Re" => "ℜ",
        "Im" => "ℑ",
        "quad" => "\u{2003}",
        "qquad" => "\u{2003}\u{2003}",
        "," | ":" | ";" | " " => "\u{2009}",
        "!" => "",
        "{" => "{",
        "}" => "}",
        "%" => "%",
        "$" => "$",
        "&" => "&",
        "#" => "#",
        "_" => "_",
        "|" => "‖",
        _ => return None,
    })
}

const FUNCTIONS: &[&str] =
    &["sin", "cos", "tan", "log", "ln", "exp", "lim", "max", "min", "sup", "inf", "det", "arg", "deg", "sec", "csc", "cot", "sinh", "cosh", "tanh", "mod", "gcd"];

const BLACKBOARD: &[(char, &str)] = &[('R', "ℝ"), ('N', "ℕ"), ('Z', "ℤ"), ('Q', "ℚ"), ('C', "ℂ"), ('P', "ℙ"), ('E', "𝔼")];

struct Parser<'a> {
    s: &'a str,
    i: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<char> {
        self.s[self.i..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.i += c.len_utf8();
        Some(c)
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn command(&mut self) -> String {
        let start = self.i;
        match self.peek() {
            Some(c) if c.is_ascii_alphabetic() => {
                while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                    self.bump();
                }
            }
            Some(_) => {
                self.bump();
            }
            None => {}
        }
        self.s[start..self.i].to_string()
    }

    /// Raw text of a `{…}` group (for `\text`).
    fn raw_group(&mut self) -> String {
        self.skip_ws();
        if self.peek() != Some('{') {
            return self.bump().map(String::from).unwrap_or_default();
        }
        self.bump();
        let start = self.i;
        let mut depth = 1;
        while let Some(c) = self.bump() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return self.s[start..self.i - 1].to_string();
                    }
                }
                _ => {}
            }
        }
        self.s[start..].to_string()
    }

    fn row(&mut self, close: Option<char>) -> Node {
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            match self.peek() {
                None => break,
                Some(c) if Some(c) == close => {
                    self.bump();
                    break;
                }
                Some('^' | '_') => {
                    let base = items.pop().unwrap_or(Node::Sym { text: String::new(), italic: false });
                    items.push(self.scripts(base));
                }
                _ => {
                    if let Some(atom) = self.atom() {
                        items.push(atom);
                    }
                }
            }
        }
        if items.len() == 1 { items.pop().unwrap() } else { Node::Row(items) }
    }

    fn scripts(&mut self, base: Node) -> Node {
        // `x_i^2` and `x^2_i` both attach to the same base.
        let (base, mut sub, mut sup) = match base {
            Node::Scripts { base, sub, sup } => (*base, sub, sup),
            other => (other, None, None),
        };
        while let Some(c @ ('^' | '_')) = {
            self.skip_ws();
            self.peek()
        } {
            self.bump();
            let arg = self.arg();
            if c == '^' {
                sup = Some(Box::new(arg));
            } else {
                sub = Some(Box::new(arg));
            }
        }
        Node::Scripts { base: Box::new(base), sub, sup }
    }

    /// One argument: a group or a single atom.
    fn arg(&mut self) -> Node {
        self.skip_ws();
        if self.peek() == Some('{') {
            self.bump();
            return self.row(Some('}'));
        }
        self.atom().unwrap_or(Node::Sym { text: String::new(), italic: false })
    }

    fn atom(&mut self) -> Option<Node> {
        let c = self.bump()?;
        Some(match c {
            '{' => self.row(Some('}')),
            '}' => return None,
            '&' => Node::Sym { text: "\u{2003}".into(), italic: false },
            '~' => Node::Sym { text: " ".into(), italic: false },
            '\\' => {
                let cmd = self.command();
                match cmd.as_str() {
                    "\\" => Node::Break,
                    "frac" | "dfrac" | "tfrac" => {
                        let a = self.arg();
                        let b = self.arg();
                        Node::Frac(Box::new(a), Box::new(b))
                    }
                    "sqrt" => {
                        self.skip_ws();
                        if self.peek() == Some('[') {
                            // Index: drop it (rare in chat math).
                            while self.bump().is_some_and(|c| c != ']') {}
                        }
                        Node::Sqrt(Box::new(self.arg()))
                    }
                    "text" | "textrm" | "mathrm" | "operatorname" | "textbf" | "mathbf" | "textit" | "mbox" => {
                        Node::Text(self.raw_group())
                    }
                    "mathbb" => {
                        let g = self.raw_group();
                        let mapped: String = g
                            .chars()
                            .map(|c| BLACKBOARD.iter().find(|(k, _)| *k == c).map_or(c.to_string(), |(_, v)| v.to_string()))
                            .collect();
                        Node::Sym { text: mapped, italic: false }
                    }
                    "mathcal" | "mathit" | "boldsymbol" | "vec" | "hat" | "bar" | "overline" | "tilde" | "dot" => {
                        let inner = self.arg();
                        match cmd.as_str() {
                            "vec" => Node::Row(vec![inner, Node::Sym { text: "\u{20D7}".into(), italic: false }]),
                            "hat" => Node::Row(vec![inner, Node::Sym { text: "\u{0302}".into(), italic: false }]),
                            "bar" | "overline" => Node::Row(vec![inner, Node::Sym { text: "\u{0304}".into(), italic: false }]),
                            "tilde" => Node::Row(vec![inner, Node::Sym { text: "\u{0303}".into(), italic: false }]),
                            "dot" => Node::Row(vec![inner, Node::Sym { text: "\u{0307}".into(), italic: false }]),
                            _ => inner,
                        }
                    }
                    "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" => {
                        self.skip_ws();
                        match self.peek() {
                            Some('\\') => {
                                self.bump();
                                let d = self.command();
                                Node::Sym { text: symbol(&d).unwrap_or("").into(), italic: false }
                            }
                            Some('.') => {
                                self.bump();
                                Node::Sym { text: String::new(), italic: false }
                            }
                            Some(_) => Node::Sym { text: self.bump().map(String::from).unwrap_or_default(), italic: false },
                            None => return None,
                        }
                    }
                    "begin" | "end" => {
                        let _ = self.raw_group();
                        Node::Sym { text: String::new(), italic: false }
                    }
                    "displaystyle" | "textstyle" | "limits" | "nolimits" => Node::Sym { text: String::new(), italic: false },
                    f if FUNCTIONS.contains(&f) => Node::Text(f.to_string()),
                    other => Node::Sym { text: symbol(other).map_or_else(|| other.to_string(), str::to_string), italic: false },
                }
            }
            c if c.is_ascii_alphabetic() => Node::Sym { text: c.to_string(), italic: true },
            c if c.is_ascii_digit() || c == '.' => {
                let mut s = c.to_string();
                while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '.') {
                    s.push(self.bump().unwrap());
                }
                Node::Sym { text: s, italic: false }
            }
            '-' => Node::Sym { text: "−".into(), italic: false },
            '*' => Node::Sym { text: "∗".into(), italic: false },
            '\'' => Node::Sym { text: "′".into(), italic: false },
            c => Node::Sym { text: c.to_string(), italic: false },
        })
    }
}

fn parse(src: &str) -> Node {
    Parser { s: src, i: 0 }.row(None)
}

fn is_op(n: &Node) -> bool {
    matches!(n, Node::Sym { text, .. } if ["=", "+", "−", "≤", "≥", "≠", "≈", "→", "⇒", "×", "·", "±", "<", ">", "∈", "≡", "÷"].contains(&text.as_str()))
}

fn el(node: &Node, size: f32, color: Hsla) -> AnyElement {
    match node {
        Node::Sym { text, italic } => div()
            .text_size(px(size))
            .line_height(px(size * 1.25))
            .when(*italic, |d| d.italic())
            .when(is_op(node), |d| d.px(px(size * 0.22)))
            .child(text.clone())
            .into_any_element(),
        Node::Text(t) => div().text_size(px(size)).line_height(px(size * 1.25)).px(px(size * 0.08)).child(t.clone()).into_any_element(),
        Node::Row(items) => {
            // `\\` splits a display block into lines.
            if items.iter().any(|n| *n == Node::Break) {
                let mut lines = vec![Vec::new()];
                for n in items {
                    if *n == Node::Break {
                        lines.push(Vec::new());
                    } else {
                        lines.last_mut().unwrap().push(n.clone());
                    }
                }
                return div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(size * 0.4))
                    .children(lines.iter().map(|l| el(&Node::Row(l.clone()), size, color)))
                    .into_any_element();
            }
            // A sign that starts a row (or follows an operator or an open
            // bracket) is unary: no operator spacing.
            div()
                .flex()
                .items_center()
                .children(items.iter().enumerate().map(|(i, n)| {
                    let unary = matches!(n, Node::Sym { text, .. } if text == "−" || text == "+" || text == "±")
                        && (i == 0
                            || is_op(&items[i - 1])
                            || matches!(&items[i - 1], Node::Sym { text, .. } if text == "(" || text == "["));
                    if unary {
                        let Node::Sym { text, .. } = n else { unreachable!() };
                        div().text_size(px(size)).line_height(px(size * 1.25)).child(text.clone()).into_any_element()
                    } else {
                        el(n, size, color)
                    }
                }))
                .into_any_element()
        }
        Node::Scripts { base, sub, sup } => {
            let small = (size * 0.68).max(9.0);
            let big = matches!(&**base, Node::Sym { text, .. } if ["∑", "∏", "∫", "∬", "∮"].contains(&text.as_str()));
            if big {
                // Limits above and below a big operator.
                return div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .px(px(size * 0.1))
                    .children(sup.as_ref().map(|s| el(s, small, color)))
                    .child(el(base, size * 1.45, color))
                    .children(sub.as_ref().map(|s| el(s, small, color)))
                    .into_any_element();
            }
            div()
                .flex()
                .items_center()
                .child(el(base, size, color))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .justify_between()
                        .h(px(size * 1.6))
                        .child(match sup {
                            Some(s) => el(s, small, color),
                            None => div().h(px(small)).into_any_element(),
                        })
                        .child(match sub {
                            Some(s) => el(s, small, color),
                            None => div().h(px(small)).into_any_element(),
                        }),
                )
                .into_any_element()
        }
        Node::Frac(a, b) => {
            let s = (size * 0.88).max(10.0);
            div()
                .flex()
                .flex_col()
                .items_center()
                .px(px(size * 0.18))
                .child(div().px(px(2.0)).child(el(a, s, color)))
                .child(div().w_full().h(px(1.2)).my(px(1.5)).bg(color))
                .child(div().px(px(2.0)).child(el(b, s, color)))
                .into_any_element()
        }
        Node::Sqrt(inner) => div()
            .flex()
            .items_center()
            .child(div().text_size(px(size * 1.2)).child("√"))
            .child(div().border_t_1().border_color(color).pt(px(1.0)).px(px(2.0)).child(el(inner, size, color)))
            .into_any_element(),
        Node::Break => div().into_any_element(),
    }
}

/// A display-math block, typeset.
pub fn block(src: &str, size: f32, color: Hsla) -> AnyElement {
    let src = src.trim().trim_start_matches("$$").trim_end_matches("$$").trim();
    let src = src.trim_start_matches("\\[").trim_end_matches("\\]");
    div()
        .font_family(MATH_FONT)
        .text_color(color)
        .flex()
        .justify_center()
        .child(el(&parse(src), size, color))
        .into_any_element()
}

#[cfg(windows)]
const MATH_FONT: &str = "Cambria Math";
#[cfg(target_os = "macos")]
const MATH_FONT: &str = "STIX Two Math";
#[cfg(not(any(windows, target_os = "macos")))]
const MATH_FONT: &str = "DejaVu Serif";

const SUP: &[(char, char)] = &[
    ('0', '⁰'), ('1', '¹'), ('2', '²'), ('3', '³'), ('4', '⁴'), ('5', '⁵'), ('6', '⁶'), ('7', '⁷'), ('8', '⁸'), ('9', '⁹'),
    ('+', '⁺'), ('-', '⁻'), ('−', '⁻'), ('=', '⁼'), ('(', '⁽'), (')', '⁾'), ('n', 'ⁿ'), ('i', 'ⁱ'), ('k', 'ᵏ'), ('x', 'ˣ'),
    ('T', 'ᵀ'), ('a', 'ᵃ'), ('b', 'ᵇ'), ('c', 'ᶜ'), ('d', 'ᵈ'), ('e', 'ᵉ'), ('m', 'ᵐ'), ('t', 'ᵗ'), ('y', 'ʸ'),
];
const SUB: &[(char, char)] = &[
    ('0', '₀'), ('1', '₁'), ('2', '₂'), ('3', '₃'), ('4', '₄'), ('5', '₅'), ('6', '₆'), ('7', '₇'), ('8', '₈'), ('9', '₉'),
    ('+', '₊'), ('-', '₋'), ('−', '₋'), ('=', '₌'), ('(', '₍'), (')', '₎'), ('a', 'ₐ'), ('e', 'ₑ'), ('i', 'ᵢ'), ('j', 'ⱼ'),
    ('k', 'ₖ'), ('m', 'ₘ'), ('n', 'ₙ'), ('o', 'ₒ'), ('p', 'ₚ'), ('r', 'ᵣ'), ('s', 'ₛ'), ('t', 'ₜ'), ('x', 'ₓ'),
];

fn flat(n: &Node) -> String {
    match n {
        Node::Sym { text, .. } => text.clone(),
        Node::Text(t) => t.clone(),
        Node::Row(items) => items.iter().map(flat).collect::<Vec<_>>().join(""),
        Node::Scripts { base, sub, sup } => {
            let mut s = flat(base);
            for (script, table, open) in [(sub, SUB, "_("), (sup, SUP, "^(")] {
                if let Some(x) = script {
                    let text = flat(x);
                    let mapped: Option<String> = text.chars().map(|c| table.iter().find(|(k, _)| *k == c).map(|(_, v)| *v)).collect();
                    match mapped {
                        Some(m) => s.push_str(&m),
                        None => {
                            s.push_str(open);
                            s.push_str(&text);
                            s.push(')');
                        }
                    }
                }
            }
            s
        }
        Node::Frac(a, b) => {
            let (a, b) = (flat(a), flat(b));
            let wrap = |x: String| if x.chars().count() > 1 { format!("({x})") } else { x };
            format!("{}⁄{}", wrap(a), wrap(b))
        }
        Node::Sqrt(x) => format!("√({})", flat(x)),
        Node::Break => "; ".into(),
    }
}

/// Inline `$…$` math as a readable Unicode line.
pub fn inline(src: &str) -> String {
    let mut out = String::new();
    let tree = parse(src.trim());
    let text = flat(&tree);
    // Space binary relations a little.
    for c in text.chars() {
        if ['=', '+', '≤', '≥', '≠', '≈', '→', '⇒'].contains(&c) {
            out.push(' ');
            out.push(c);
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fractions_and_scripts() {
        let n = parse(r"\frac{a}{b} + x^{2}_i");
        let Node::Row(items) = n else { panic!("row") };
        assert!(matches!(items[0], Node::Frac(..)));
        assert!(matches!(items[2], Node::Scripts { sub: Some(_), sup: Some(_), .. }));
    }

    #[test]
    fn inline_uses_unicode_scripts() {
        assert_eq!(inline(r"x^2 + y_1"), "x² + y₁");
        assert_eq!(inline(r"t_n = 100 \cdot 2^{n}"), "tₙ = 100·2ⁿ");
        assert_eq!(inline(r"\frac{1}{2}"), "1⁄2");
        assert_eq!(inline(r"e^{i\pi}"), "eⁱ^(π)".replace("ⁱ^(π)", "^(iπ)"));
        assert_eq!(inline(r"\alpha \le \beta"), "α ≤ β");
    }

    #[test]
    fn text_and_functions() {
        assert_eq!(inline(r"\sin x \text{ ms}"), "sinx ms");
    }
}
