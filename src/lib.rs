//! Snake: a small game, steered by buttons or swipes.

#![no_std]

extern crate alloc;

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::string::String;

use core::time::Duration;

use paperpaw_sdk::builtin::{actions, menus};
use paperpaw_sdk::realtime::ScreenMode;
use paperpaw_sdk::setting::{Setting, Text};
use paperpaw_sdk::ui::{self, UiStyle};
use paperpaw_sdk::{
    ActionSpec, Arg, Canvas, Color, Command, Cx, Direction, Env, Gesture, GestureKind, Handled,
    Id, Manifest, MenuEntry, Params, Plugin, Rect, RegisterError, Registrar, Screen, ScreenSpec,
    SettingGroupSpec, SettingSpec, Version,
};

pub const PLUGIN: Id = Id::new_static("snake");
pub const OPEN: Id = Id::new_static("snake.open");
pub const SCREEN: Id = Id::new_static("snake.screen");

const COLS: i32 = 16;
const ROWS: i32 = 20;
/// How fast the snake moves: one of [`SPEEDS`].
pub const SPEED: Setting<Text> = Setting::new("snake.speed");
const SPEEDS: [&str; 4] = [
    "snake.speed.slow",
    "snake.speed.normal",
    "snake.speed.fast",
    "snake.speed.turbo",
];

/// The settings page of the game, under Customise.
const GROUP: Id = Id::new_static("snake.group");
static GROUP_SPEC: SettingGroupSpec = SettingGroupSpec::new(GROUP, "snake.group").order(260);

static SETTINGS: [SettingSpec; 1] = [SettingSpec::choice(
    SPEED.static_str(),
    "snake.setting.speed",
    &SPEEDS,
    SPEEDS[1],
    GROUP,
)];

/// Milliseconds between steps, as the speed setting says, but never less
/// than the panel needs to show a frame (as measured on the device), so a
/// fast setting runs as fast as the panel allows instead of piling up.
fn step_ms(env: &Env<'_>) -> u64 {
    let wanted = match env.settings().read(&SPEED) {
        "snake.speed.slow" => 700,
        "snake.speed.fast" => 250,
        "snake.speed.turbo" => 150,
        _ => 450,
    };
    let panel = env
        .panel()
        .fastest()
        .map_or(0, |(_, time)| time.as_millis() as u64);
    wanted.max(panel + panel / 4)
}

#[derive(Debug, Default)]
pub struct Snake;

static HOW: [(&str, &str); 2] = [
    ("en", include_str!("../how/en.txt")),
    ("de", include_str!("../how/de.txt")),
];

impl Plugin for Snake {
    fn manifest(&self) -> Manifest {
        Manifest::new(PLUGIN, "Snake", Version::new(0, 1, 1))
            .description("The classic snake game.")
            .how(&HOW)
            .uses(paperpaw_sdk::Uses::NOTHING)
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        r.add_action(ActionSpec::new(OPEN, "snake.title").handler(|_, _, out| {
            out.push(Command::OpenScreen(SCREEN, Params::new()));
        }))?;
        r.add_menu_entry(MenuEntry::new(menus::TOOLS, OPEN).order(91))?;
        r.add_icon(OPEN, Cow::Borrowed("star"))?;
        r.add_screen(ScreenSpec::new(
            SCREEN,
            "snake.title",
            Box::new(|env, _| Box::new(Game::new(env))),
        ))?;
        r.add_setting_group(&GROUP_SPEC)?;
        r.add_icon(GROUP, Cow::Borrowed("star"))?;
        r.add_settings(&SETTINGS)?;
        r.add_translations(Cow::Borrowed(include_str!("../i18n/en.ini")))?;
        r.add_translations(Cow::Borrowed(include_str!("../i18n/de.ini")))?;
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dir {
    Up,
    Right,
    Down,
    Left,
}

impl Dir {
    fn delta(self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::Right => (1, 0),
            Dir::Down => (0, 1),
            Dir::Left => (-1, 0),
        }
    }
    fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::Right => Dir::Left,
            Dir::Down => Dir::Up,
            Dir::Left => Dir::Right,
        }
    }
    fn turn_left(self) -> Dir {
        match self {
            Dir::Up => Dir::Left,
            Dir::Left => Dir::Down,
            Dir::Down => Dir::Right,
            Dir::Right => Dir::Up,
        }
    }
    fn turn_right(self) -> Dir {
        self.turn_left().opposite()
    }
}

struct Game {
    style: UiStyle,
    header: ui::Header,
    body: VecDeque<(i32, i32)>,
    dir: Dir,
    next: Dir,
    food: (i32, i32),
    score: u32,
    best: u32,
    over: bool,
    started: bool,
    seed: u32,
    /// The step time in milliseconds as set, told to the screen's mode.
    pace: u64,
    /// When the next step is due: steps come at a fixed pace.
    due: paperpaw_sdk::Instant,
}

impl Game {
    fn new(env: &Env<'_>) -> Self {
        let style = UiStyle::new(env);
        let mut g = Self {
            header: ui::Header::new(env, &style, true),
            style,
            body: VecDeque::new(),
            dir: Dir::Right,
            next: Dir::Right,
            food: (0, 0),
            score: 0,
            best: 0,
            over: false,
            started: false,
            seed: (env.now().millis() as u32) | 1,
            pace: step_ms(env),
            due: env.now(),
        };
        g.reset();
        g
    }

    fn reset(&mut self) {
        self.body.clear();
        for i in 0..3 {
            self.body.push_back((COLS / 2 - i, ROWS / 2));
        }
        self.dir = Dir::Right;
        self.next = Dir::Right;
        self.score = 0;
        self.over = false;
        self.started = false;
        self.place_food();
    }

    fn rand(&mut self) -> u32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed
    }

    fn place_food(&mut self) {
        loop {
            let x = (self.rand() % COLS as u32) as i32;
            let y = (self.rand() % ROWS as u32) as i32;
            if !self.body.contains(&(x, y)) {
                self.food = (x, y);
                return;
            }
            if self.body.len() as i32 >= COLS * ROWS {
                return;
            }
        }
    }

    fn step(&mut self) {
        if self.over {
            return;
        }
        self.dir = self.next;
        let (dx, dy) = self.dir.delta();
        let (hx, hy) = self.body[0];
        let head = (hx + dx, hy + dy);
        let eating = head == self.food;
        // The tail moves away this step unless the snake grows.
        let len = self.body.len();
        let hit_self = self
            .body
            .iter()
            .take(if eating { len } else { len - 1 })
            .any(|&p| p == head);
        if head.0 < 0 || head.1 < 0 || head.0 >= COLS || head.1 >= ROWS || hit_self {
            self.over = true;
            return;
        }
        self.body.push_front(head);
        if eating {
            self.score += 1;
            if self.score > self.best {
                self.best = self.score;
            }
            self.place_food();
        } else {
            self.body.pop_back();
        }
    }

    fn steer(&mut self, d: Dir) {
        if d != self.dir.opposite() {
            self.next = d;
        }
    }

    /// Starts the game, or restarts it after the end.
    fn start(&mut self, cx: &mut Cx<'_, '_>) {
        if self.over {
            self.reset();
        }
        self.started = true;
        let ms = step_ms(cx.env());
        self.pace = ms;
        self.due = cx.env().now().add_millis(ms);
        cx.wake_at(self.due);
        cx.request_redraw();
    }

    /// Plans the next step at a fixed pace: one step time after the last
    /// one was due, not after it was drawn. When the panel is slower than
    /// that, the missed steps are dropped, not caught up.
    fn plan_next(&mut self, cx: &mut Cx<'_, '_>) {
        let ms = step_ms(cx.env());
        self.pace = ms;
        let now = cx.env().now();
        let mut due = self.due.add_millis(ms);
        if due.millis() <= now.millis() {
            due = now.add_millis(ms);
        }
        self.due = due;
        cx.wake_at(due);
    }
}

impl Screen for Game {
    /// While playing: no page refresh cleaning, the fast refresh, black and
    /// white, and the reader stays awake.
    fn mode(&self) -> ScreenMode {
        // Read at each input and frame. What the panel is asked for holds
        // all the while the screen is open; only the pace follows the game.
        let mode = ScreenMode::new()
            .cleans_itself()
            .fastest()
            .grays(2)
            .awake_for(Duration::from_secs(300));
        if self.started && !self.over {
            mode.frame_every(Duration::from_millis(self.pace))
        } else {
            mode
        }
    }

    fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
        if self.started && !self.over {
            self.plan_next(cx);
        }
    }

    fn on_timer(&mut self, cx: &mut Cx<'_, '_>) {
        if !self.started || self.over {
            return;
        }
        self.step();
        if !self.over {
            self.plan_next(cx);
        }
        cx.request_redraw();
        cx.request_refresh(paperpaw_sdk::RefreshMode::Mono);
    }

    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let mut ui = ui::Ui::new(canvas, env, &self.style);
        let i18n = env.i18n();
        let note = i18n.format("snake.score", &[("score", Arg::Int(i64::from(self.score)))]);
        self.header.draw(&mut ui, i18n.tr("snake.title"), Some(&note), None);

        let display = env.device().display;
        let top = i32::from(self.header.area().height);
        let margin = i32::from(self.style.margin());
        let avail_w = i32::from(display.width) - 2 * margin;
        let avail_h = i32::from(display.height) - top - 3 * margin;
        let cell = (avail_w / COLS).min(avail_h / ROWS).max(4);
        let bw = cell * COLS;
        let bh = cell * ROWS;
        let ox = (i32::from(display.width) - bw) / 2;
        let oy = top + margin;

        let canvas = ui.canvas();
        // Board frame.
        let b = Color::BLACK;
        canvas.fill_rect(ox - 2, oy - 2, (bw + 4) as u32, 2, b);
        canvas.fill_rect(ox - 2, oy + bh, (bw + 4) as u32, 2, b);
        canvas.fill_rect(ox - 2, oy - 2, 2, (bh + 4) as u32, b);
        canvas.fill_rect(ox + bw, oy - 2, 2, (bh + 4) as u32, b);
        // Food: a smaller square.
        let (fx, fy) = self.food;
        canvas.fill_rect(ox + fx * cell + cell / 4, oy + fy * cell + cell / 4,
            (cell / 2) as u32, (cell / 2) as u32, b);
        // Snake.
        for (i, &(x, y)) in self.body.iter().enumerate() {
            let pad = if i == 0 { 0 } else { 1 };
            canvas.fill_rect(ox + x * cell + pad, oy + y * cell + pad,
                (cell - 2 * pad) as u32, (cell - 2 * pad) as u32, b);
        }

        // Message under the board.
        let msg_top = oy + bh + margin / 2;
        let rest = i32::from(display.height) - msg_top;
        if rest > 0 {
            let key = if self.over {
                "snake.over"
            } else if self.started {
                "snake.playing"
            } else {
                "snake.start"
            };
            let text: String = if self.over {
                i18n.format("snake.over", &[("best", Arg::Int(i64::from(self.best)))])
            } else {
                String::from(i18n.tr(key))
            };
            let area = Rect::new(0, msg_top as _, display.width, rest as _);
            ui.message(area, &text);
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        if *action == actions::BACK {
            cx.command(Command::Back);
            return Handled::Yes;
        }
        if *action == actions::CONFIRM {
            if !self.started || self.over {
                self.start(cx);
            }
            return Handled::Yes;
        }
        let turn = if *action == actions::LEFT || *action == actions::UP {
            Some(true)
        } else if *action == actions::RIGHT || *action == actions::DOWN {
            Some(false)
        } else {
            None
        };
        if let Some(left) = turn {
            if !self.started || self.over {
                self.start(cx);
            } else if left {
                self.steer(self.dir.turn_left());
            } else {
                self.steer(self.dir.turn_right());
            }
            return Handled::Yes;
        }
        Handled::No
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        match gesture.kind {
            GestureKind::Swipe(d) => {
                if !self.started || self.over {
                    self.start(cx);
                }
                self.steer(match d {
                    Direction::Up => Dir::Up,
                    Direction::Down => Dir::Down,
                    Direction::Left => Dir::Left,
                    Direction::Right => Dir::Right,
                });
                Handled::Yes
            }
            GestureKind::Tap => {
                if let Some(action) = self.header.tap(cx.env(), &self.style, gesture.start, None) {
                    cx.command(Command::run(action));
                } else if !self.started || self.over {
                    self.start(cx);
                } else {
                    // Tap on the left or right half turns that way.
                    let half = i32::from(cx.env().device().display.width) / 2;
                    if (gesture.start.x as i32) < half {
                        self.steer(self.dir.turn_left());
                    } else {
                        self.steer(self.dir.turn_right());
                    }
                }
                Handled::Yes
            }
            _ => Handled::No,
        }
    }
}
