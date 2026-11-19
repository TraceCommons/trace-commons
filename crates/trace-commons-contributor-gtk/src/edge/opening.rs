//! Opening and closing the rail, in the order that keeps the rail inside
//! its surface: the surface grows before the rail draws open into it, and
//! shrinks only once the rail has finished sliding out.
//!
//! A port of the Mac's `EdgeRailOpening` with its tests
//! (`EdgeRailControllerTests`). There the rail used to draw open into the
//! four-point closed panel for a turn, and the panel shrank under it while
//! it was still going (#1517). Here the same order keeps a layer surface
//! from clipping the slide: it is sized before the revealer opens and after
//! it has closed. Both steps run on the next turn of the main loop, never
//! inside the pointer event that asked.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

type Step = Box<dyn FnOnce()>;

/// Runs a step on the next turn of the main loop.
pub type NextTurn = Box<dyn Fn(Step)>;
/// Slides the rail to its (already set) open or closed state, then calls
/// the completion once a closing rail has gone.
pub type Animate = Box<dyn Fn(bool, Step)>;
/// Sizes the surface for an open (`true`) or closed rail.
pub type Place = Box<dyn Fn(bool)>;

pub struct Opening {
    /// Whether the rail is drawn open.
    open: Cell<bool>,
    /// What was asked for last. A step queued for the other answer finds
    /// it changed and does nothing.
    wanted: Cell<bool>,
    next_turn: NextTurn,
    animate: Animate,
    place: Place,
    this: RefCell<Weak<Opening>>,
}

impl Opening {
    pub fn new(next_turn: NextTurn, animate: Animate, place: Place) -> Rc<Self> {
        let opening = Rc::new(Self {
            open: Cell::new(false),
            wanted: Cell::new(false),
            next_turn,
            animate,
            place,
            this: RefCell::new(Weak::new()),
        });
        *opening.this.borrow_mut() = Rc::downgrade(&opening);
        opening
    }

    /// Whether the rail is drawn open.
    pub fn is_open(&self) -> bool {
        self.open.get()
    }

    /// Opens or closes the rail.
    pub fn set(&self, open: bool) {
        self.wanted.set(open);
        let this = self.this.borrow().clone();
        if open {
            (self.next_turn)(Box::new(move || {
                let Some(this) = this.upgrade() else { return };
                if !this.wanted.get() || this.open.get() {
                    return;
                }
                (this.place)(true);
                this.open.set(true);
                (this.animate)(true, Box::new(|| {}));
            }));
        } else if self.open.get() {
            self.open.set(false);
            (self.animate)(
                false,
                Box::new(move || {
                    let Some(outer) = this.upgrade() else { return };
                    let this = Rc::downgrade(&outer);
                    (outer.next_turn)(Box::new(move || {
                        let Some(this) = this.upgrade() else { return };
                        if this.wanted.get() || this.open.get() {
                            return;
                        }
                        (this.place)(false);
                    }));
                }),
            );
        }
    }

    /// Closed at once, as a new or removed surface starts.
    pub fn reset(&self) {
        self.wanted.set(false);
        self.open.set(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Recorded {
        Place(bool),
        Rail(bool),
    }

    /// An `Opening` whose main-loop turns and animation completions run
    /// only when the test says, recording each step in order.
    struct Recorder {
        opening: Rc<Opening>,
        steps: Rc<RefCell<Vec<Recorded>>>,
        turns: Rc<RefCell<Vec<Step>>>,
        completions: Rc<RefCell<Vec<Step>>>,
    }

    impl Recorder {
        fn new() -> Self {
            let steps = Rc::new(RefCell::new(Vec::new()));
            let turns: Rc<RefCell<Vec<Step>>> = Rc::new(RefCell::new(Vec::new()));
            let completions: Rc<RefCell<Vec<Step>>> = Rc::new(RefCell::new(Vec::new()));
            let opening = Opening::new(
                {
                    let turns = Rc::clone(&turns);
                    Box::new(move |step| turns.borrow_mut().push(step))
                },
                {
                    let steps = Rc::clone(&steps);
                    let completions = Rc::clone(&completions);
                    Box::new(move |open, completion| {
                        steps.borrow_mut().push(Recorded::Rail(open));
                        completions.borrow_mut().push(completion);
                    })
                },
                {
                    let steps = Rc::clone(&steps);
                    Box::new(move |open| steps.borrow_mut().push(Recorded::Place(open)))
                },
            );
            Self {
                opening,
                steps,
                turns,
                completions,
            }
        }

        fn run_turns(&self) {
            loop {
                let next = {
                    let mut turns = self.turns.borrow_mut();
                    if turns.is_empty() {
                        break;
                    }
                    turns.remove(0)
                };
                next();
            }
        }

        fn finish_animations(&self) {
            let done: Vec<Step> = self.completions.borrow_mut().drain(..).collect();
            for completion in done {
                completion();
            }
        }

        fn steps(&self) -> Vec<Recorded> {
            self.steps.borrow().clone()
        }

        fn clear(&self) {
            self.steps.borrow_mut().clear();
        }
    }

    #[test]
    fn opening_grows_the_surface_before_the_rail_draws_open() {
        let rail = Recorder::new();
        rail.opening.set(true);
        assert_eq!(rail.steps(), [], "moved inside the caller's event");
        assert!(!rail.opening.is_open());

        rail.run_turns();
        assert_eq!(rail.steps(), [Recorded::Place(true), Recorded::Rail(true)]);
        assert!(rail.opening.is_open());
    }

    #[test]
    fn closing_shrinks_the_surface_only_after_the_rail_has_gone() {
        let rail = Recorder::new();
        rail.opening.set(true);
        rail.run_turns();
        rail.clear();

        rail.opening.set(false);
        assert_eq!(rail.steps(), [Recorded::Rail(false)]);
        rail.run_turns();
        assert_eq!(
            rail.steps(),
            [Recorded::Rail(false)],
            "shrank under the rail while it was going"
        );

        rail.finish_animations();
        rail.run_turns();
        assert_eq!(
            rail.steps(),
            [Recorded::Rail(false), Recorded::Place(false)]
        );
    }

    #[test]
    fn reopening_while_closing_keeps_the_surface_open() {
        let rail = Recorder::new();
        rail.opening.set(true);
        rail.run_turns();
        rail.opening.set(false);
        rail.clear();

        rail.opening.set(true);
        rail.run_turns();
        rail.finish_animations();
        rail.run_turns();
        assert_eq!(rail.steps(), [Recorded::Place(true), Recorded::Rail(true)]);
        assert!(rail.opening.is_open());
    }

    #[test]
    fn closing_before_the_open_turn_leaves_the_rail_closed() {
        let rail = Recorder::new();
        rail.opening.set(true);
        rail.opening.set(false);
        rail.run_turns();
        rail.finish_animations();
        rail.run_turns();
        assert_eq!(rail.steps(), []);
        assert!(!rail.opening.is_open());
    }

    #[test]
    fn opening_an_open_rail_does_nothing() {
        let rail = Recorder::new();
        rail.opening.set(true);
        rail.run_turns();
        rail.opening.set(true);
        rail.run_turns();
        assert_eq!(rail.steps(), [Recorded::Place(true), Recorded::Rail(true)]);
    }

    #[test]
    fn reset_closes_at_once_without_a_step() {
        let rail = Recorder::new();
        rail.opening.set(true);
        rail.run_turns();
        rail.clear();
        rail.opening.reset();
        assert!(!rail.opening.is_open());
        rail.run_turns();
        assert_eq!(rail.steps(), []);
    }
}
