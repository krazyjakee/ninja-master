//! Shared menu viewport. Coordinates use the original 320 x 224 UI canvas.
use crate::{
    render::Frame,
    text::{Font, Style},
};

#[derive(Clone, Copy, Debug)]
pub enum MenuEvent {
    Hover(i32, i32),
    Click(i32, i32, bool),
    Scroll(i32),
    Back,
}

#[derive(Clone, Copy, Debug)]
pub struct Menu {
    pub selected: usize,
    pub first: usize,
    pub top: i32,
    pub visible: usize,
    pub row_height: i32,
}

impl Menu {
    pub fn new(top: i32, visible: usize, row_height: i32) -> Self {
        Self {
            selected: 0,
            first: 0,
            top,
            visible,
            row_height,
        }
    }
    pub fn reveal(&mut self, count: usize) {
        self.selected = self.selected.min(count.saturating_sub(1));
        if self.selected < self.first {
            self.first = self.selected;
        }
        if self.selected >= self.first + self.visible {
            self.first = self.selected + 1 - self.visible;
        }
        self.first = self.first.min(count.saturating_sub(self.visible));
    }
    pub fn step(&mut self, delta: i32, count: usize) {
        if count == 0 {
            return;
        }
        self.selected = (self.selected as i32 + delta).rem_euclid(count as i32) as usize;
        self.reveal(count);
    }
    pub fn scroll(&mut self, delta: i32, count: usize) {
        self.first = (self.first as i32 + delta).clamp(0, count.saturating_sub(self.visible) as i32)
            as usize;
        self.selected = self.selected.clamp(
            self.first,
            (self.first + self.visible).min(count).saturating_sub(1),
        );
    }
    pub fn hit(&self, x: i32, y: i32, count: usize) -> Option<usize> {
        if !(24..296).contains(&x) || y < self.top {
            return None;
        }
        let row = ((y - self.top) / self.row_height) as usize;
        (row < self.visible && self.first + row < count).then_some(self.first + row)
    }
    pub fn draw(&self, frame: &mut Frame, font: &Font, rows: &[String]) {
        let height = self.visible as i32 * self.row_height;
        frame.rect(16, self.top - 10, 288, height + 20, [8, 12, 24]);
        let white = Style::plain([220, 224, 232], 1);
        for (row, text) in rows.iter().enumerate().skip(self.first).take(self.visible) {
            let y = self.top + (row - self.first) as i32 * self.row_height;
            if row == self.selected {
                frame.rect(22, y, 276, self.row_height, [48, 56, 80]);
            }
            font.draw_text(frame, 28, y + (self.row_height - 8) / 2, text, white);
        }
        if self.first > 0 {
            font.draw_text(frame, 280, self.top - 9, "^", white);
        }
        if self.first + self.visible < rows.len() {
            font.draw_text(frame, 280, self.top + height + 1, "V", white);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_menus_wrap_scroll_and_hit_only_visible_rows() {
        let mut menu = Menu::new(40, 6, 20);
        menu.step(-1, 20);
        assert_eq!((menu.selected, menu.first), (19, 14));
        assert_eq!(menu.hit(24, 40, 20), Some(14));
        assert_eq!(menu.hit(24, 160, 20), None);
        assert_eq!(menu.hit(0, 40, 20), None);
        menu.step(1, 20);
        assert_eq!((menu.selected, menu.first), (0, 0));
        menu.scroll(3, 20);
        assert_eq!((menu.selected, menu.first), (3, 3));
        menu.reveal(2);
        assert_eq!((menu.selected, menu.first), (1, 0));
    }
}
