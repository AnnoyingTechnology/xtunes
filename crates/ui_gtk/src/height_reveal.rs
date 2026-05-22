// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 AnnoyingTechnology

use std::{cell::Cell, time::Duration};

use gtk::{glib, prelude::*, subclass::prelude::*};

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, Default)]
    pub struct HeightReveal {
        pub(super) child: RefCell<Option<gtk::Widget>>,
        pub(super) progress: Cell<f64>,
        pub(super) animation_generation: Cell<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for HeightReveal {
        const NAME: &'static str = "XtunesHeightReveal";
        type Type = super::HeightReveal;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for HeightReveal {
        fn dispose(&self) {
            if let Some(child) = self.child.take() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for HeightReveal {
        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.child.borrow().clone() else {
                return (0, 0, -1, -1);
            };

            let (minimum, natural, minimum_baseline, natural_baseline) =
                child.measure(orientation, for_size);
            if orientation == gtk::Orientation::Horizontal {
                return (minimum, natural, minimum_baseline, natural_baseline);
            }

            let height = interpolated_height(natural, self.progress.get());
            (height, height, -1, -1)
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let Some(child) = self.child.borrow().clone() else {
                return;
            };

            let (_, natural_height, _, _) = child.measure(gtk::Orientation::Vertical, width);
            child.allocate(width, natural_height.max(height), -1, None);
        }
    }
}

glib::wrapper! {
    pub struct HeightReveal(ObjectSubclass<imp::HeightReveal>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl HeightReveal {
    pub(crate) fn new(child: &impl IsA<gtk::Widget>) -> Self {
        let reveal: Self = glib::Object::new();
        reveal.set_overflow(gtk::Overflow::Hidden);
        reveal.set_child(child);
        reveal
    }

    pub(crate) fn reveal(&self, animate: bool, duration: Duration) {
        let generation = self.next_animation_generation();
        if !animate || duration.is_zero() {
            self.set_progress(1.0);
            return;
        }

        self.set_progress(0.0);

        let reveal = self.downgrade();
        let started_at = Cell::new(None);
        self.add_tick_callback(move |_widget, clock| {
            let Some(reveal) = reveal.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if reveal.imp().animation_generation.get() != generation {
                return glib::ControlFlow::Break;
            }

            let start = started_at.get().unwrap_or_else(|| {
                let frame_time = clock.frame_time();
                started_at.set(Some(frame_time));
                frame_time
            });
            let elapsed = clock.frame_time().saturating_sub(start) as f64;
            let duration = duration.as_micros() as f64;
            let progress = (elapsed / duration).clamp(0.0, 1.0);
            reveal.set_progress(progress);

            if progress >= 1.0 {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn set_child(&self, child: &impl IsA<gtk::Widget>) {
        let child = child.as_ref();
        if let Some(current_child) = self.imp().child.replace(Some(child.clone())) {
            current_child.unparent();
        }
        child.set_parent(self);
    }

    fn next_animation_generation(&self) -> u64 {
        let generation = self.imp().animation_generation.get().wrapping_add(1);
        self.imp().animation_generation.set(generation);
        generation
    }

    fn set_progress(&self, progress: f64) {
        self.imp().progress.set(progress.clamp(0.0, 1.0));
        self.queue_resize();
    }
}

fn interpolated_height(natural_height: i32, progress: f64) -> i32 {
    ((natural_height.max(0) as f64) * progress.clamp(0.0, 1.0)).round() as i32
}

#[cfg(test)]
mod tests {
    use super::interpolated_height;

    #[test]
    fn interpolated_height_clamps_progress() {
        assert_eq!(interpolated_height(200, -1.0), 0);
        assert_eq!(interpolated_height(200, 0.5), 100);
        assert_eq!(interpolated_height(200, 2.0), 200);
    }

    #[test]
    fn interpolated_height_uses_linear_progress() {
        assert_eq!(interpolated_height(200, 0.25), 50);
        assert_eq!(interpolated_height(200, 0.75), 150);
    }
}
