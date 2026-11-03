//! Rider's colour picker over a swatch (Passenger's ColourPicker): a square of saturation and brightness for the hue
//! chosen on the bar under it, an alpha bar, and the colour before and after beside its hex, which the field below
//! keeps in step. Its gradients are the colours being chosen, not the theme's; the editor's popup hosts it and gives
//! it the pointer.

use crate::theme;
use gpui::{
    Bounds, IntoElement, Pixels, Point, Rgba, Styled, canvas, div, linear_color_stop, linear_gradient, prelude::*, px,
};
use std::cell::Cell;
use std::rc::Rc;

/// What the pointer drags across.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Area {
    Shade,
    Hue,
    Alpha,
}

/// The colour being chosen, as hue (0–360), saturation and value (0–1) and alpha, with the colour the swatch had.
pub struct Pick {
    pub h: f32,
    pub s: f32,
    pub v: f32,
    pub a: u8,
    start: [u8; 4],
    /// Where each area was last painted, for the pointer.
    placed: Rc<[Cell<Option<Bounds<Pixels>>>; 3]>,
    pub dragging: Option<Area>,
}

impl Pick {
    /// From `#AARRGGBB`, as the host writes a swatch's colour.
    pub fn new(argb: [u8; 4]) -> Self {
        let (h, s, v) = to_hsv(argb[1], argb[2], argb[3]);
        Self { h, s, v, a: argb[0], start: argb, placed: Rc::default(), dragging: None }
    }

    /// The colour as alpha, red, green and blue.
    pub fn argb(&self) -> [u8; 4] {
        let (r, g, b) = from_hsv(self.h, self.s, self.v);
        [self.a, r, g, b]
    }

    /// As XAML writes a colour: `#RRGGBB`, its alpha first when it is not opaque.
    pub fn hex(&self) -> String {
        let [a, r, g, b] = self.argb();
        if a == 255 { format!("#{r:02X}{g:02X}{b:02X}") } else { format!("#{a:02X}{r:02X}{g:02X}{b:02X}") }
    }

    /// Takes a colour as typed (`#RGB`, `#RRGGBB`, `#AARRGGBB`), once it reads as one and differs from this.
    pub fn take_typed(&mut self, text: &str) -> bool {
        match parse(text) {
            Some(argb) if argb != self.argb() => {
                let (h, s, v) = to_hsv(argb[1], argb[2], argb[3]);
                // A grey has no hue of its own: the hue chosen stays.
                if s > 0. {
                    self.h = h;
                }
                (self.s, self.v, self.a) = (s, v, argb[0]);
                true
            }
            _ => false,
        }
    }

    /// The area under a point, if any.
    pub fn area_at(&self, point: Point<Pixels>) -> Option<Area> {
        [Area::Shade, Area::Hue, Area::Alpha]
            .into_iter()
            .find(|area| self.bounds(*area).is_some_and(|b| b.contains(&point)))
    }

    fn bounds(&self, area: Area) -> Option<Bounds<Pixels>> {
        self.placed[area as usize].get()
    }

    /// The colour at a point of `area`, the point held inside it, as a press or drag there chooses it.
    pub fn choose(&mut self, area: Area, point: Point<Pixels>) {
        let Some(bounds) = self.bounds(area) else { return };
        let x = (f32::from(point.x - bounds.origin.x) / f32::from(bounds.size.width)).clamp(0., 1.);
        let y = (f32::from(point.y - bounds.origin.y) / f32::from(bounds.size.height)).clamp(0., 1.);
        match area {
            Area::Shade => (self.s, self.v) = (x, 1. - y),
            Area::Hue => self.h = (x * 360.).min(359.9),
            Area::Alpha => self.a = (x * 255.).round() as u8,
        }
    }

    /// The square, the hue bar and the alpha bar, each as tall as given, with the swatches of before and after.
    pub fn areas(&self) -> impl IntoElement {
        let colour = rgba(self.argb());
        let opaque = rgba([255, colour_byte(colour.r), colour_byte(colour.g), colour_byte(colour.b)]);
        let hue = {
            let (r, g, b) = from_hsv(self.h, 1., 1.);
            rgba([255, r, g, b])
        };
        let (s, v, h, a) = (self.s, self.v, self.h, self.a);
        let placed = self.placed.clone();
        let record = move |area: Area| {
            let placed = placed.clone();
            move |bounds: Bounds<Pixels>| placed[area as usize].set(Some(bounds))
        };
        let shade = {
            let record = record(Area::Shade);
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    record(bounds);
                    let white = rgba([255, 255, 255, 255]);
                    let black = rgba([255, 0, 0, 0]);
                    window.paint_quad(gpui::fill(bounds, hue));
                    window.paint_quad(gpui::fill(
                        bounds,
                        linear_gradient(
                            90.,
                            linear_color_stop(white, 0.),
                            linear_color_stop(rgba([0, 255, 255, 255]), 1.),
                        ),
                    ));
                    window.paint_quad(gpui::fill(
                        bounds,
                        linear_gradient(180., linear_color_stop(rgba([0, 0, 0, 0]), 0.), linear_color_stop(black, 1.)),
                    ));
                    marker(window, bounds, s, 1. - v);
                },
            )
            .w_full()
            .h(px(160.))
        };
        let hue_bar = {
            let record = record(Area::Hue);
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    record(bounds);
                    // Six stretches between the primaries and secondaries, each a gradient of its own.
                    let width = bounds.size.width / 6.;
                    for i in 0..6 {
                        let (r0, g0, b0) = from_hsv(i as f32 * 60., 1., 1.);
                        let (r1, g1, b1) = from_hsv(((i + 1) * 60 % 360) as f32, 1., 1.);
                        let part = Bounds::new(
                            bounds.origin + gpui::point(width * i as f32, px(0.)),
                            gpui::size(width, bounds.size.height),
                        );
                        window.paint_quad(gpui::fill(
                            part,
                            linear_gradient(
                                90.,
                                linear_color_stop(rgba([255, r0, g0, b0]), 0.),
                                linear_color_stop(rgba([255, r1, g1, b1]), 1.),
                            ),
                        ));
                    }
                    marker(window, bounds, h / 360., 0.5);
                },
            )
            .w_full()
            .h(px(14.))
        };
        let alpha_bar = {
            let record = record(Area::Alpha);
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    record(bounds);
                    checker(window, bounds);
                    let clear = Rgba { a: 0., ..opaque };
                    window.paint_quad(gpui::fill(
                        bounds,
                        linear_gradient(90., linear_color_stop(clear, 0.), linear_color_stop(opaque, 1.)),
                    ));
                    marker(window, bounds, a as f32 / 255., 0.5);
                },
            )
            .w_full()
            .h(px(14.))
        };
        let swatch = |fill: Rgba| {
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    checker(window, bounds);
                    window.paint_quad(gpui::fill(bounds, fill));
                },
            )
            .size(px(26.))
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().debug_selector(|| "colour square".into()).child(shade))
            .child(div().debug_selector(|| "colour hue".into()).child(hue_bar))
            .child(div().debug_selector(|| "colour alpha".into()).child(alpha_bar))
            .child(div().flex().child(swatch(rgba(self.start))).child(swatch(colour)))
    }
}

/// A ring at a fraction of an area's width and height, light over dark so it shows on any colour.
fn marker(window: &mut gpui::Window, bounds: Bounds<Pixels>, x: f32, y: f32) {
    let at = bounds.origin + gpui::point(bounds.size.width * x, bounds.size.height * y);
    for (grow, colour) in [(px(6.), theme::colour_marker_shadow()), (px(5.), theme::colour_marker())] {
        let ring = Bounds::new(at - gpui::point(grow, grow), gpui::size(grow * 2., grow * 2.));
        window.paint_quad(gpui::outline(ring, colour, gpui::BorderStyle::Solid).corner_radii(grow));
    }
}

/// A checkerboard under what is clear.
fn checker(window: &mut gpui::Window, bounds: Bounds<Pixels>) {
    window.paint_quad(gpui::fill(bounds, theme::colour_checker_light()));
    let step = px(7.);
    let (columns, rows) = ((bounds.size.width / step).ceil() as usize, (bounds.size.height / step).ceil() as usize);
    for row in 0..rows {
        for column in (row % 2..columns).step_by(2) {
            let origin = bounds.origin + gpui::point(step * column as f32, step * row as f32);
            let size = gpui::size(step.min(bounds.right() - origin.x), step.min(bounds.bottom() - origin.y));
            window.paint_quad(gpui::fill(Bounds::new(origin, size), theme::colour_checker_dark()));
        }
    }
}

fn rgba([a, r, g, b]: [u8; 4]) -> Rgba {
    Rgba { r: r as f32 / 255., g: g as f32 / 255., b: b as f32 / 255., a: a as f32 / 255. }
}

fn colour_byte(value: f32) -> u8 {
    (value * 255.).round() as u8
}

/// `#RGB`, `#RRGGBB` or `#AARRGGBB` as alpha, red, green and blue; none for anything else (a name).
pub fn parse(text: &str) -> Option<[u8; 4]> {
    let digits = text.trim().strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(digits.get(i..i + 2)?, 16).ok();
    match digits.len() {
        3 => {
            let nibble = |i: usize| u8::from_str_radix(digits.get(i..i + 1)?, 16).ok().map(|n| n * 17);
            Some([255, nibble(0)?, nibble(1)?, nibble(2)?])
        }
        6 => Some([255, byte(0)?, byte(2)?, byte(4)?]),
        8 => Some([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
        _ => None,
    }
}

fn to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255., g as f32 / 255., b as f32 / 255.);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let delta = max - min;
    let hue = if delta == 0. {
        0.
    } else if max == r {
        60. * ((g - b) / delta).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / delta + 2.)
    } else {
        60. * ((r - g) / delta + 4.)
    };
    (hue.rem_euclid(360.), if max == 0. { 0. } else { delta / max }, max)
}

fn from_hsv(hue: f32, saturation: f32, value: f32) -> (u8, u8, u8) {
    let c = value * saturation;
    let x = c * (1. - ((hue / 60.).rem_euclid(2.) - 1.).abs());
    let m = value - c;
    let (r, g, b) = match hue {
        h if h < 60. => (c, x, 0.),
        h if h < 120. => (x, c, 0.),
        h if h < 180. => (0., c, x),
        h if h < 240. => (0., x, c),
        h if h < 300. => (x, 0., c),
        _ => (c, 0., x),
    };
    let byte = |v: f32| ((v + m) * 255.).round() as u8;
    (byte(r), byte(g), byte(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_reads_back_through_hue_saturation_and_value() {
        for argb in [[255, 0xE2, 0xA3, 0x3F], [0x80, 0x12, 0x34, 0x56], [255, 0, 0, 0], [255, 255, 255, 255]] {
            assert_eq!(Pick::new(argb).argb(), argb, "a colour did not survive the picker unchanged");
        }
        assert_eq!(Pick::new([255, 0xE2, 0xA3, 0x3F]).hex(), "#E2A33F");
        assert_eq!(Pick::new([0x80, 0x12, 0x34, 0x56]).hex(), "#80123456");
        assert_eq!(parse("#fa0"), Some([255, 0xFF, 0xAA, 0]));
        assert_eq!(parse("Red"), None, "a name is the host's to read");
    }
}
