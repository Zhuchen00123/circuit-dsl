//! Source locations and the source map used to render diagnostics.
//!
//! Offsets are byte offsets into a UTF-8 source file. Conversions to
//! line/column happen only when rendering, and count Unicode scalar values
//! (not bytes) so that carets line up under non-ASCII text.

use std::fmt;
use std::sync::Arc;

/// Identifies one source file within a [`SourceMap`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(pub u32);

impl fmt::Debug for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "src{}", self.0)
    }
}

/// A half-open byte range `[start, end)` in one source file.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceSpan {
    pub source: SourceId,
    pub start: u32,
    pub end: u32,
}

impl SourceSpan {
    pub const fn new(source: SourceId, start: u32, end: u32) -> Self {
        Self { source, start, end }
    }

    /// A span that does not point anywhere real.
    ///
    /// Used for IR nodes synthesized during elaboration (for example a node
    /// created by a loop) where there is no single honest source location.
    /// Diagnostics must not fabricate a location for these; they should fall
    /// back to naming the enclosing definition instead.
    pub const fn synthetic() -> Self {
        Self {
            source: SourceId(u32::MAX),
            start: 0,
            end: 0,
        }
    }

    pub const fn is_synthetic(&self) -> bool {
        self.source.0 == u32::MAX
    }

    /// Smallest span covering both. Spans from different files cannot be
    /// merged; the left-hand span wins in that case.
    pub fn merge(self, other: Self) -> Self {
        if self.source != other.source {
            return self;
        }
        Self {
            source: self.source,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub const fn len(&self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub const fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

impl fmt::Debug for SourceSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_synthetic() {
            return write!(f, "<synthetic>");
        }
        write!(f, "{}..{}", self.start, self.end)
    }
}

/// A value paired with the source location it came from.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Spanned<T> {
    pub node: T,
    pub span: SourceSpan,
}

impl<T> Spanned<T> {
    pub const fn new(node: T, span: SourceSpan) -> Self {
        Self { node, span }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            node: f(self.node),
            span: self.span,
        }
    }

    pub fn as_ref(&self) -> Spanned<&T> {
        Spanned {
            node: &self.node,
            span: self.span,
        }
    }
}

/// One loaded source file.
pub struct SourceFile {
    pub id: SourceId,
    /// Display name, e.g. `examples/rc_filter.cdsl`.
    pub name: String,
    pub text: Arc<str>,
    /// Byte offset of the start of each line.
    line_starts: Vec<u32>,
}

impl SourceFile {
    fn new(id: SourceId, name: String, text: Arc<str>) -> Self {
        let mut line_starts = vec![0u32];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        Self {
            id,
            name,
            text,
            line_starts,
        }
    }

    /// 1-based line and 1-based column (in Unicode scalar values).
    pub fn line_col(&self, offset: u32) -> (u32, u32) {
        let offset = offset.min(self.text.len() as u32);
        // Index of the last line start <= offset.
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let line_start = self.line_starts[line_idx] as usize;
        let prefix = &self.text[line_start..offset as usize];
        let col = prefix.chars().count() as u32 + 1;
        (line_idx as u32 + 1, col)
    }

    /// Text of a 1-based line, without the trailing newline.
    pub fn line_text(&self, line: u32) -> &str {
        let idx = line.saturating_sub(1) as usize;
        let Some(&start) = self.line_starts.get(idx) else {
            return "";
        };
        let end = self
            .line_starts
            .get(idx + 1)
            .map(|&e| e as usize)
            .unwrap_or(self.text.len());
        self.text[start as usize..end].trim_end_matches(['\n', '\r'])
    }

    /// Column of the caret line, in display characters.
    ///
    /// Tabs are expanded so that the caret does not drift; the returned
    /// leading text is what should be printed before the carets.
    fn caret_prefix(&self, offset: u32) -> (String, usize) {
        let (line, col) = self.line_col(offset);
        let text = self.line_text(line);
        let mut out = String::new();
        for ch in text.chars().take(col as usize - 1) {
            match ch {
                '\t' => out.push_str("    "),
                ' ' => out.push(' '),
                // Replace any other char with a space of the same column
                // width so the caret stays aligned under multi-byte text.
                _ => out.push(' '),
            }
        }
        let width = out.chars().count();
        (out, width)
    }
}

/// All loaded source files, and the renderer for diagnostics.
#[derive(Default)]
pub struct SourceMap {
    files: Vec<SourceFile>,
}

impl SourceMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, name: impl Into<String>, text: impl Into<Arc<str>>) -> SourceId {
        let id = SourceId(self.files.len() as u32);
        self.files
            .push(SourceFile::new(id, name.into(), text.into()));
        id
    }

    pub fn file(&self, id: SourceId) -> Option<&SourceFile> {
        self.files.get(id.0 as usize)
    }

    pub fn file_name(&self, id: SourceId) -> &str {
        self.file(id)
            .map(|f| f.name.as_str())
            .unwrap_or("<unknown>")
    }

    /// Source text covered by a span.
    pub fn snippet(&self, span: SourceSpan) -> &str {
        match self.file(span.source) {
            Some(f) => f
                .text
                .get(span.start as usize..span.end as usize)
                .unwrap_or(""),
            None => "",
        }
    }

    /// Render the `--> file:line:col` location line.
    pub fn location(&self, span: SourceSpan) -> String {
        match self.file(span.source) {
            Some(f) => {
                let (line, col) = f.line_col(span.start);
                format!("{}:{}:{}", f.name, line, col)
            }
            None => "<unknown>".to_string(),
        }
    }

    /// Render a source excerpt with a caret underline, as used in the
    /// diagnostic examples in the language specification.
    pub fn render_excerpt(&self, span: SourceSpan, label: Option<&str>) -> Option<String> {
        let file = self.file(span.source)?;
        let (line_no, _) = file.line_col(span.start);
        let text = file.line_text(line_no);
        let gutter_w = line_no.to_string().len();
        let (prefix, _) = file.caret_prefix(span.start);

        // Caret width: at least one, capped to the rest of the line.
        let rest = text
            .chars()
            .skip(prefix.chars().count())
            .collect::<String>();
        let caret_chars = if span.is_empty() {
            1
        } else {
            self.snippet(span).chars().count().max(1)
        };
        let caret_chars = caret_chars.min(rest.chars().count().max(1));
        let carets = "^".repeat(caret_chars);

        // `line | source text`, then the gutter and the carets, matching the
        // layout fixed in docs/language.md §8.
        let mut out = String::new();
        out.push_str(&format!("{:>w$} | {}\n", line_no, text, w = gutter_w));
        out.push_str(&format!("{:>w$} | {}{}", "", prefix, carets, w = gutter_w));
        if let Some(l) = label {
            out.push(' ');
            out.push_str(l);
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_is_one_based() {
        let mut sm = SourceMap::new();
        let id = sm.add("t.cdsl", "abc\ndef\n");
        let f = sm.file(id).unwrap();
        assert_eq!(f.line_col(0), (1, 1));
        assert_eq!(f.line_col(2), (1, 3));
        assert_eq!(f.line_col(4), (2, 1));
        assert_eq!(f.line_col(6), (2, 3));
    }

    #[test]
    fn line_col_counts_chars_not_bytes() {
        let mut sm = SourceMap::new();
        // 'Ω' is two bytes in UTF-8, so byte 2 is the start of a character
        // that sits in display column 3.
        let id = sm.add("t.cdsl", "1.Ω\n");
        let f = sm.file(id).unwrap();
        assert_eq!(f.line_col(0), (1, 1));
        assert_eq!(f.line_col(1), (1, 2));
        assert_eq!(f.line_col(2), (1, 3));
        assert_eq!(f.line_text(1), "1.Ω");
    }

    #[test]
    fn merge_takes_union_within_one_file() {
        let s = SourceId(0);
        let a = SourceSpan::new(s, 10, 20);
        let b = SourceSpan::new(s, 15, 30);
        assert_eq!(a.merge(b), SourceSpan::new(s, 10, 30));
    }

    #[test]
    fn merge_across_files_keeps_left() {
        let a = SourceSpan::new(SourceId(0), 10, 20);
        let b = SourceSpan::new(SourceId(1), 0, 5);
        assert_eq!(a.merge(b), a);
    }

    #[test]
    fn synthetic_spans_are_flagged() {
        assert!(SourceSpan::synthetic().is_synthetic());
        assert!(!SourceSpan::new(SourceId(0), 0, 1).is_synthetic());
    }

    #[test]
    fn excerpt_renders_line_and_carets() {
        let mut sm = SourceMap::new();
        let id = sm.add("t.cdsl", "one\ntwo three\nfour\n");
        // "three" starts at byte 4 + 4 = 8
        let span = SourceSpan::new(id, 8, 13);
        let out = sm.render_excerpt(span, Some("here")).unwrap();
        assert!(out.contains("two three"), "{out}");
        assert!(out.contains("^^^^^"), "{out}");
        assert!(out.ends_with("here"), "{out}");
    }
}
