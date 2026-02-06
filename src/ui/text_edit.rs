pub struct TextEditState;

impl TextEditState {
    pub fn select_all(
        text: &str,
        cursor: &mut usize,
        selection: &mut Option<(usize, usize)>,
        anchor: &mut Option<usize>,
    ) {
        let len = text.chars().count();
        *selection = Some((0, len));
        *anchor = Some(0);
        *cursor = len;
    }

    pub fn normalized_selection(selection: Option<(usize, usize)>) -> Option<(usize, usize)> {
        selection.map(|(a, b)| if a <= b { (a, b) } else { (b, a) })
    }

    pub fn has_selection(selection: Option<(usize, usize)>) -> bool {
        matches!(Self::normalized_selection(selection), Some((a, b)) if a != b)
    }

    pub fn clear_selection(selection: &mut Option<(usize, usize)>, anchor: &mut Option<usize>) {
        *selection = None;
        *anchor = None;
    }

    pub fn set_selection_from_anchor(
        selection: &mut Option<(usize, usize)>,
        anchor_ref: &mut Option<usize>,
        anchor: usize,
        cursor: usize,
    ) {
        *anchor_ref = Some(anchor);
        *selection = Some((anchor, cursor));
    }

    pub fn split_at_cursor(text: &str, cursor: usize) -> (String, String) {
        let mut left = String::new();
        let mut right = String::new();
        for (i, ch) in text.chars().enumerate() {
            if i < cursor {
                left.push(ch);
            } else {
                right.push(ch);
            }
        }
        (left, right)
    }

    pub fn delete_selection_if_any(
        text: &mut String,
        cursor: &mut usize,
        selection: &mut Option<(usize, usize)>,
        anchor: &mut Option<usize>,
    ) -> bool {
        let Some((a, b)) = Self::normalized_selection(*selection) else {
            return false;
        };
        if a == b {
            return false;
        }
        let mut out = String::new();
        for (i, ch) in text.chars().enumerate() {
            if i < a || i >= b {
                out.push(ch);
            }
        }
        *text = out;
        *cursor = a;
        Self::clear_selection(selection, anchor);
        true
    }

    pub fn insert_text(
        text: &mut String,
        cursor: &mut usize,
        selection: &mut Option<(usize, usize)>,
        anchor: &mut Option<usize>,
        insert: &str,
    ) {
        Self::delete_selection_if_any(text, cursor, selection, anchor);
        let (left, right) = Self::split_at_cursor(text, *cursor);
        let mut out = left;
        out.push_str(insert);
        out.push_str(&right);
        *text = out;
        let max = text.chars().count();
        *cursor = (*cursor + insert.chars().count()).min(max);
        Self::clear_selection(selection, anchor);
    }

    pub fn pop_char_before_cursor(
        text: &mut String,
        cursor: &mut usize,
        selection: &mut Option<(usize, usize)>,
        anchor: &mut Option<usize>,
    ) {
        if *cursor == 0 {
            return;
        }
        let mut out = String::new();
        for (i, ch) in text.chars().enumerate() {
            if i + 1 == *cursor {
                continue;
            }
            out.push(ch);
        }
        *text = out;
        *cursor = cursor.saturating_sub(1);
        Self::clear_selection(selection, anchor);
    }
}
