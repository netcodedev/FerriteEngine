//! Text editing model of a single-line field: caret, selection and edits,
//! independent of rendering. Positions are char indices.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextEdit {
    text: String,
    cursor: usize,
    /// Other end of the selection; `None` (or equal to `cursor`) means no
    /// selection.
    anchor: Option<usize>,
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl TextEdit {
    pub fn new(text: &str) -> Self {
        Self {
            cursor: text.chars().count(),
            text: text.to_owned(),
            anchor: None,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, index: usize) -> usize {
        self.text
            .char_indices()
            .nth(index)
            .map_or(self.text.len(), |(b, _)| b)
    }

    /// Replace the whole text, caret at the end, no selection.
    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_owned();
        self.cursor = self.len();
        self.anchor = None;
    }

    /// `(start, end)` of the selection, if any.
    pub fn selection(&self) -> Option<(usize, usize)> {
        match self.anchor {
            Some(a) if a != self.cursor => Some((a.min(self.cursor), a.max(self.cursor))),
            _ => None,
        }
    }

    pub fn selected_text(&self) -> Option<&str> {
        let (s, e) = self.selection()?;
        Some(&self.text[self.byte(s)..self.byte(e)])
    }

    /// Move the caret; with `select` the selection grows from where it was.
    pub fn set_cursor(&mut self, index: usize, select: bool) {
        let index = index.min(self.len());
        if select {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        self.cursor = index;
    }

    pub fn select_all(&mut self) {
        self.anchor = Some(0);
        self.cursor = self.len();
    }

    /// Select the word (or run of non-word chars) around `index`.
    pub fn select_word_at(&mut self, index: usize) {
        let chars: Vec<char> = self.text.chars().collect();
        if chars.is_empty() {
            return;
        }
        let i = index.min(chars.len() - 1);
        let kind = is_word(chars[i]);
        let mut start = i;
        while start > 0 && is_word(chars[start - 1]) == kind {
            start -= 1;
        }
        let mut end = i + 1;
        while end < chars.len() && is_word(chars[end]) == kind {
            end += 1;
        }
        self.anchor = Some(start);
        self.cursor = end;
    }

    fn word_left(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i > 0 && !is_word(chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(chars[i - 1]) {
            i -= 1;
        }
        i
    }

    fn word_right(&self) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i < chars.len() && !is_word(chars[i]) {
            i += 1;
        }
        while i < chars.len() && is_word(chars[i]) {
            i += 1;
        }
        i
    }

    pub fn move_left(&mut self, select: bool, word: bool) {
        if !select {
            if let Some((start, _)) = self.selection() {
                self.set_cursor(start, false);
                return;
            }
        }
        let target = if word {
            self.word_left()
        } else {
            self.cursor.saturating_sub(1)
        };
        self.set_cursor(target, select);
    }

    pub fn move_right(&mut self, select: bool, word: bool) {
        if !select {
            if let Some((_, end)) = self.selection() {
                self.set_cursor(end, false);
                return;
            }
        }
        let target = if word { self.word_right() } else { self.cursor + 1 };
        self.set_cursor(target, select);
    }

    pub fn home(&mut self, select: bool) {
        self.set_cursor(0, select);
    }

    pub fn end(&mut self, select: bool) {
        self.set_cursor(self.len(), select);
    }

    /// Remove the selection; returns whether there was one.
    fn delete_selection(&mut self) -> bool {
        let Some((s, e)) = self.selection() else {
            return false;
        };
        let (bs, be) = (self.byte(s), self.byte(e));
        self.text.replace_range(bs..be, "");
        self.cursor = s;
        self.anchor = None;
        true
    }

    /// Type or paste `s`, replacing the selection.
    pub fn insert(&mut self, s: &str) {
        self.delete_selection();
        let b = self.byte(self.cursor);
        self.text.insert_str(b, s);
        self.cursor += s.chars().count();
    }

    pub fn backspace(&mut self, word: bool) {
        if self.delete_selection() || self.cursor == 0 {
            return;
        }
        let start = if word {
            self.word_left()
        } else {
            self.cursor - 1
        };
        let (bs, be) = (self.byte(start), self.byte(self.cursor));
        self.text.replace_range(bs..be, "");
        self.cursor = start;
    }

    pub fn delete(&mut self, word: bool) {
        if self.delete_selection() || self.cursor >= self.len() {
            return;
        }
        let end = if word {
            self.word_right()
        } else {
            self.cursor + 1
        };
        let (bs, be) = (self.byte(self.cursor), self.byte(end));
        self.text.replace_range(bs..be, "");
    }

    /// Cut the selection, returning it.
    pub fn cut(&mut self) -> Option<String> {
        let text = self.selected_text()?.to_owned();
        self.delete_selection();
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::TextEdit;

    #[test]
    fn caret_moves_and_inserts_in_the_middle() {
        let mut e = TextEdit::new("0.5");
        e.move_left(false, false);
        e.insert("2");
        assert_eq!(e.text(), "0.25");
        assert_eq!(e.cursor(), 3);
        e.home(false);
        e.insert("-");
        assert_eq!(e.text(), "-0.25");
    }

    #[test]
    fn selection_is_replaced_and_deleted() {
        let mut e = TextEdit::new("forward");
        e.home(false);
        e.move_right(true, false);
        e.move_right(true, false);
        e.move_right(true, false);
        assert_eq!(e.selected_text(), Some("for"));
        e.insert("back");
        assert_eq!(e.text(), "backward");
        e.select_all();
        e.backspace(false);
        assert_eq!(e.text(), "");
    }

    #[test]
    fn word_jumps_and_deletes() {
        let mut e = TextEdit::new("left - right");
        e.move_left(false, true);
        assert_eq!(e.cursor(), 7);
        // Like common editors: the previous word goes with its separators.
        e.backspace(true);
        assert_eq!(e.text(), "right");
        let mut e = TextEdit::new("a b");
        e.home(false);
        e.delete(true);
        assert_eq!(e.text(), " b");
    }

    #[test]
    fn collapsing_a_selection_moves_to_its_edge() {
        let mut e = TextEdit::new("abcdef");
        e.select_word_at(2);
        assert_eq!(e.selection(), Some((0, 6)));
        e.move_left(false, false);
        assert_eq!((e.cursor(), e.selection()), (0, None));
    }

    #[test]
    fn handles_multibyte_chars() {
        let mut e = TextEdit::new("aüb");
        e.move_left(false, false);
        e.backspace(false);
        assert_eq!(e.text(), "ab");
        e.insert("é");
        assert_eq!(e.text(), "aéb");
    }
}
