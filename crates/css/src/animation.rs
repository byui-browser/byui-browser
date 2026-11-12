//! CSS Animations Level 1: `@keyframes`, the `animation-*` properties, and
//! sampling animated values at a moment of the document timeline.
//!
//! The cascade calls [`apply_animations`] for each element after resolving
//! CSS-wide keywords. Animated values replace the element's normal
//! declarations but lose to `!important` ones (CSS Cascade 4 §6.1), and they
//! are applied before inheritance passes values to children, so an animated
//! `color` is inherited like any other.
//!
//! Timing follows Web Animations 1 §4 ("Timing model") with every animation's
//! start time at the origin of the document timeline, so `time` is the time
//! since the document was first styled. Keyframe intervals are eased with the
//! keyframe's `animation-timing-function`, or the element's.
//!
//! Interpolation (CSS Values 4 §3): colors interpolate in premultiplied sRGB;
//! numbers, percentages, and lengths interpolate component-wise in
//! space-separated values such as `"10px 20%"`. Lengths with different units
//! are converted to CSS pixels when the units are absolute or font-relative.
//! Every other pair of values, including `auto` and keywords, flips
//! discretely halfway through the interval.
//!
//! Limitations:
//! - Animations start when the document is first styled, not when an element
//!   starts matching `animation-name`, because the cascade keeps no state
//!   between runs. A `paused` animation therefore holds its frame at time 0.
//! - Animation events, `animation-composition`, `animation-timeline`, and the
//!   `linear()` easing function are not supported.
//! - Viewport-relative lengths and percentages only interpolate against the
//!   same unit; mixed with other units they animate discretely.
//! - `@keyframes` inside conditional rules such as `@media` are ignored, as
//!   the cascade does not apply those rules yet.

use std::collections::{BTreeMap, HashMap};

use crate::color::{CssColor, parse_color};
use crate::rule_index::compile_declarations;
use crate::shorthands::split_components;
use crate::stylesheet::{Declaration, Stylesheet};
use crate::values::{self, LengthContext, parse_length, parse_number, parse_percentage};

/// The `animation-*` longhands the `animation` shorthand sets, in the order
/// [`expand_shorthand`] returns them.
const LONGHANDS: [&str; 8] = [
    "animation-name",
    "animation-duration",
    "animation-timing-function",
    "animation-delay",
    "animation-iteration-count",
    "animation-direction",
    "animation-fill-mode",
    "animation-play-state",
];

/// Initial values of [`LONGHANDS`], in the same order.
const INITIAL_LONGHANDS: [&str; 8] = ["none", "0s", "ease", "0s", "1", "normal", "none", "running"];

const CSS_WIDE_KEYWORDS: [&str; 5] = ["inherit", "initial", "unset", "revert", "revert-layer"];

/// An easing function from CSS Easing 1, mapping input progress to output
/// progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum TimingFunction {
    /// `linear`: output equals input.
    Linear,
    /// `cubic-bezier(x1, y1, x2, y2)` and the keywords defined with it.
    CubicBezier(f64, f64, f64, f64),
    /// `steps(count, position)`, `step-start`, and `step-end`.
    Steps(u32, StepPosition),
}

/// The `<step-position>` of a `steps()` easing function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepPosition {
    /// `jump-start` (or `start`): the first jump happens at the start.
    Start,
    /// `jump-end` (or `end`): the last jump happens at the end.
    End,
    /// `jump-none`: no jump at either end.
    Neither,
    /// `jump-both`: jumps at both the start and the end.
    Both,
}

impl TimingFunction {
    /// The initial `animation-timing-function`, `ease`.
    const EASE: Self = Self::CubicBezier(0.25, 0.1, 0.25, 1.0);

    /// Parses one `<easing-function>`, or returns `None` if unsupported.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        let keyword = match text.as_str() {
            "linear" => Some(Self::Linear),
            "ease" => Some(Self::EASE),
            "ease-in" => Some(Self::CubicBezier(0.42, 0.0, 1.0, 1.0)),
            "ease-out" => Some(Self::CubicBezier(0.0, 0.0, 0.58, 1.0)),
            "ease-in-out" => Some(Self::CubicBezier(0.42, 0.0, 0.58, 1.0)),
            "step-start" => Some(Self::Steps(1, StepPosition::Start)),
            "step-end" => Some(Self::Steps(1, StepPosition::End)),
            _ => None,
        };
        if keyword.is_some() {
            return keyword;
        }
        if let Some(arguments) = function_arguments(&text, "cubic-bezier") {
            let numbers = arguments
                .iter()
                .map(|argument| parse_number(argument).map(f64::from))
                .collect::<Option<Vec<_>>>()?;
            let [x1, y1, x2, y2] = numbers[..] else {
                return None;
            };
            let valid = (0.0..=1.0).contains(&x1)
                && (0.0..=1.0).contains(&x2)
                && y1.is_finite()
                && y2.is_finite();
            return valid.then_some(Self::CubicBezier(x1, y1, x2, y2));
        }
        if let Some(arguments) = function_arguments(&text, "steps") {
            let (count, position) = match arguments.as_slice() {
                [count] => (count, "end"),
                [count, position] => (count, position.as_str()),
                _ => return None,
            };
            let count: u32 = count.parse().ok()?;
            let position = match position {
                "jump-start" | "start" => StepPosition::Start,
                "jump-end" | "end" => StepPosition::End,
                "jump-none" => StepPosition::Neither,
                "jump-both" => StepPosition::Both,
                _ => return None,
            };
            let minimum = if position == StepPosition::Neither {
                2
            } else {
                1
            };
            return (count >= minimum).then_some(Self::Steps(count, position));
        }
        None
    }

    /// Maps input progress `t` (normally `0.0..=1.0`) to output progress.
    pub(crate) fn apply(self, t: f64) -> f64 {
        match self {
            Self::Linear => t,
            Self::CubicBezier(x1, y1, x2, y2) => cubic_bezier(x1, y1, x2, y2, t),
            // CSS Easing 1 §3.1, ignoring the "before flag", which only
            // matters for fill-backwards with a jump at the start.
            Self::Steps(count, position) => {
                let count = f64::from(count);
                let mut step = (t * count).floor();
                if matches!(position, StepPosition::Start | StepPosition::Both) {
                    step += 1.0;
                }
                let jumps = match position {
                    StepPosition::Start | StepPosition::End => count,
                    StepPosition::Neither => count - 1.0,
                    StepPosition::Both => count + 1.0,
                };
                if t >= 0.0 && step < 0.0 {
                    step = 0.0;
                }
                if t <= 1.0 && step > jumps {
                    step = jumps;
                }
                step / jumps
            }
        }
    }
}

/// Evaluates a CSS cubic Bézier easing curve at input progress `t`.
///
/// The curve runs from (0, 0) to (1, 1) with control points (x1, y1) and
/// (x2, y2). Solves x(s) = t for the curve parameter `s` with Newton's
/// method, falling back to bisection, and returns y(s).
fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, t: f64) -> f64 {
    if t <= 0.0 || t >= 1.0 {
        return t.clamp(0.0, 1.0);
    }
    let curve = |a: f64, b: f64, s: f64| {
        let inverse = 1.0 - s;
        3.0 * inverse * inverse * s * a + 3.0 * inverse * s * s * b + s * s * s
    };
    let slope = |a: f64, b: f64, s: f64| {
        let inverse = 1.0 - s;
        3.0 * inverse * inverse * a + 6.0 * inverse * s * (b - a) + 3.0 * s * s * (1.0 - b)
    };
    let mut s = t;
    for _ in 0..8 {
        let error = curve(x1, x2, s) - t;
        if error.abs() < 1e-7 {
            return curve(y1, y2, s);
        }
        let derivative = slope(x1, x2, s);
        if derivative.abs() < 1e-6 {
            break;
        }
        s -= error / derivative;
    }
    let (mut low, mut high) = (0.0, 1.0);
    s = t;
    for _ in 0..50 {
        let x = curve(x1, x2, s);
        if (x - t).abs() < 1e-7 {
            break;
        }
        if x < t {
            low = s;
        } else {
            high = s;
        }
        s = (low + high) / 2.0;
    }
    curve(y1, y2, s)
}

/// Returns the comma-separated arguments of `name(...)`, trimmed.
fn function_arguments(text: &str, name: &str) -> Option<Vec<String>> {
    let inner = text
        .strip_prefix(name)?
        .trim_start()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    Some(
        inner
            .split(',')
            .map(|part| part.trim().to_owned())
            .collect(),
    )
}

/// Splits a value at top-level commas, keeping commas inside parentheses and
/// quoted strings. Each item is trimmed.
pub(crate) fn split_list(value: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut depth = 0_usize;
    let mut quote = None;
    let mut start = 0;
    for (index, character) in value.char_indices() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(character),
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, ',') if depth == 0 => {
                items.push(value[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    items.push(value[start..].trim());
    items
}

/// Parses a `<time>` such as `1.5s` or `200ms` into seconds.
fn parse_time(text: &str) -> Option<f64> {
    let lower = text.trim().to_ascii_lowercase();
    let (number, scale) = if let Some(number) = lower.strip_suffix("ms") {
        (number, 0.001)
    } else {
        (lower.strip_suffix('s')?, 1.0)
    };
    let seconds = f64::from(parse_number(number)?) * scale;
    seconds.is_finite().then_some(seconds)
}

/// Parses a `<keyframes-name>`: a `<custom-ident>` or a string. Returns the
/// name without quotes, or `None` for `none`, CSS-wide keywords, and
/// malformed names.
pub(crate) fn parse_keyframes_name(text: &str) -> Option<String> {
    let text = text.trim();
    for quote in ['"', '\''] {
        if let Some(inner) = text
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return (!inner.is_empty()).then(|| inner.to_owned());
        }
    }
    let valid = !text.is_empty()
        && !text.starts_with(|c: char| c.is_ascii_digit())
        && text
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || !c.is_ascii());
    let reserved = text.eq_ignore_ascii_case("none")
        || text.eq_ignore_ascii_case("default")
        || CSS_WIDE_KEYWORDS
            .iter()
            .any(|keyword| text.eq_ignore_ascii_case(keyword));
    (valid && !reserved).then(|| text.to_owned())
}

/// Expands the `animation` shorthand (CSS Animations 1 §3.10) into the eight
/// `animation-*` longhands, each a comma-separated list with one entry per
/// animation. Returns `None` when the value is invalid.
///
/// Within each animation, the first `<time>` is the duration and the second
/// the delay. A keyword that could be a fill mode, direction, or play state
/// is taken as such before it is taken as the animation name.
pub(crate) fn expand_shorthand(value: &str) -> Option<Vec<Declaration>> {
    let trimmed = value.trim();
    if CSS_WIDE_KEYWORDS
        .iter()
        .any(|keyword| trimmed.eq_ignore_ascii_case(keyword))
    {
        return Some(
            LONGHANDS
                .iter()
                .map(|property| Declaration {
                    property: (*property).to_owned(),
                    value: trimmed.to_owned(),
                })
                .collect(),
        );
    }
    let mut columns: [Vec<String>; 8] = Default::default();
    for layer in split_list(trimmed) {
        let mut slots: [Option<String>; 8] = Default::default();
        for part in split_components(layer) {
            let lower = part.to_ascii_lowercase();
            let slot = if parse_time(part).is_some() {
                if slots[1].is_none() { 1 } else { 3 }
            } else if TimingFunction::parse(part).is_some() {
                2
            } else if lower == "infinite" || parse_number(part).is_some_and(|n| n >= 0.0) {
                4
            } else if ["normal", "reverse", "alternate", "alternate-reverse"]
                .contains(&lower.as_str())
                && slots[5].is_none()
            {
                5
            } else if ["none", "forwards", "backwards", "both"].contains(&lower.as_str())
                && slots[6].is_none()
            {
                6
            } else if ["running", "paused"].contains(&lower.as_str()) && slots[7].is_none() {
                7
            } else if lower == "none" || parse_keyframes_name(part).is_some() {
                0
            } else {
                return None;
            };
            if slots[slot].replace(part.to_owned()).is_some() {
                return None;
            }
        }
        for (index, slot) in slots.into_iter().enumerate() {
            columns[index].push(slot.unwrap_or_else(|| INITIAL_LONGHANDS[index].to_owned()));
        }
    }
    Some(
        LONGHANDS
            .iter()
            .zip(columns)
            .map(|(property, values)| Declaration {
                property: (*property).to_owned(),
                value: values.join(", "),
            })
            .collect(),
    )
}

/// How an animation plays successive iterations (`animation-direction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Normal,
    Reverse,
    Alternate,
    AlternateReverse,
}

/// Which side of the active interval an animation fills
/// (`animation-fill-mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FillMode {
    None,
    Forwards,
    Backwards,
    Both,
}

/// One animation an element runs: the `n`th entry of each `animation-*` list.
#[derive(Debug, Clone, PartialEq)]
struct Animation {
    name: String,
    /// Iteration duration in seconds.
    duration: f64,
    timing: TimingFunction,
    /// Seconds from the timeline origin to the start of the first iteration.
    /// May be negative, which starts the animation partway through.
    delay: f64,
    /// Number of iterations; may be fractional or infinite.
    iterations: f64,
    direction: Direction,
    fill: FillMode,
    paused: bool,
}

/// The result of sampling an animation's timing at one moment.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    /// Progress through the current iteration after applying the direction,
    /// from 0 to 1, or `None` when the animation has no effect right now.
    progress: Option<f64>,
    /// Whether the animation's output can still change as time advances.
    running: bool,
}

impl Animation {
    /// Reads the element's animations from its cascaded `animation-*` values.
    ///
    /// `animation-name` decides how many animations run; the other lists are
    /// repeated or truncated to match (CSS Animations 1 §3.2). A list that
    /// fails to parse uses the property's initial value.
    fn list(properties: &BTreeMap<String, String>) -> Vec<Self> {
        let Some(names) = properties.get("animation-name") else {
            return Vec::new();
        };
        let names = split_list(names);
        fn longhand<T>(
            properties: &BTreeMap<String, String>,
            property: &str,
            initial: T,
            parse: impl Fn(&str) -> Option<T>,
        ) -> Vec<T> {
            properties
                .get(property)
                .and_then(|value| split_list(value).into_iter().map(&parse).collect())
                .filter(|values: &Vec<T>| !values.is_empty())
                .unwrap_or_else(|| vec![initial])
        }
        let durations = longhand(properties, "animation-duration", 0.0, |text| {
            if text.eq_ignore_ascii_case("auto") {
                return Some(0.0);
            }
            parse_time(text).filter(|seconds| *seconds >= 0.0)
        });
        let timings = longhand(
            properties,
            "animation-timing-function",
            TimingFunction::EASE,
            TimingFunction::parse,
        );
        let delays = longhand(properties, "animation-delay", 0.0, parse_time);
        let iterations = longhand(properties, "animation-iteration-count", 1.0, |text| {
            if text.eq_ignore_ascii_case("infinite") {
                return Some(f64::INFINITY);
            }
            parse_number(text)
                .map(f64::from)
                .filter(|count| *count >= 0.0)
        });
        let directions = longhand(
            properties,
            "animation-direction",
            Direction::Normal,
            |text| match text.to_ascii_lowercase().as_str() {
                "normal" => Some(Direction::Normal),
                "reverse" => Some(Direction::Reverse),
                "alternate" => Some(Direction::Alternate),
                "alternate-reverse" => Some(Direction::AlternateReverse),
                _ => None,
            },
        );
        let fills = longhand(
            properties,
            "animation-fill-mode",
            FillMode::None,
            |text| match text.to_ascii_lowercase().as_str() {
                "none" => Some(FillMode::None),
                "forwards" => Some(FillMode::Forwards),
                "backwards" => Some(FillMode::Backwards),
                "both" => Some(FillMode::Both),
                _ => None,
            },
        );
        let paused = longhand(properties, "animation-play-state", false, |text| match text
            .to_ascii_lowercase()
            .as_str()
        {
            "running" => Some(false),
            "paused" => Some(true),
            _ => None,
        });
        names
            .iter()
            .enumerate()
            .filter_map(|(index, name)| {
                Some(Self {
                    name: parse_keyframes_name(name)?,
                    duration: durations[index % durations.len()],
                    timing: timings[index % timings.len()],
                    delay: delays[index % delays.len()],
                    iterations: iterations[index % iterations.len()],
                    direction: directions[index % directions.len()],
                    fill: fills[index % fills.len()],
                    paused: paused[index % paused.len()],
                })
            })
            .collect()
    }

    /// Samples the animation `time` seconds after the timeline origin
    /// (Web Animations 1 §4.6–4.9 with a start time of zero, no end delay,
    /// and a playback rate of one).
    fn sample(&self, time: f64) -> Sample {
        let time = if self.paused { 0.0 } else { time };
        let active_duration = if self.duration == 0.0 || self.iterations == 0.0 {
            0.0
        } else {
            self.duration * self.iterations
        };
        let end_time = (self.delay + active_duration).max(0.0);
        let before_active = self.delay.min(end_time).max(0.0);
        let active_after = (self.delay + active_duration).min(end_time).max(0.0);
        let local = time - self.delay;
        #[derive(PartialEq)]
        enum Phase {
            Before,
            Active,
            After,
        }
        let phase = if time < before_active {
            Phase::Before
        } else if time >= active_after {
            Phase::After
        } else {
            Phase::Active
        };
        let running = !self.paused
            && match phase {
                Phase::Before => true,
                Phase::Active => active_duration > 0.0,
                Phase::After => false,
            };
        let fills_backwards = matches!(self.fill, FillMode::Backwards | FillMode::Both);
        let fills_forwards = matches!(self.fill, FillMode::Forwards | FillMode::Both);
        let active_time = match phase {
            Phase::Before if fills_backwards => 0.0,
            Phase::Active => local,
            Phase::After if fills_forwards => local.min(active_duration).max(0.0),
            _ => {
                return Sample {
                    progress: None,
                    running,
                };
            }
        };
        let overall = if self.duration == 0.0 {
            if phase == Phase::Before {
                0.0
            } else {
                self.iterations
            }
        } else {
            active_time / self.duration
        };
        let mut simple = if overall.is_infinite() {
            0.0
        } else {
            overall % 1.0
        };
        if simple == 0.0
            && phase != Phase::Before
            && active_time == active_duration
            && self.iterations != 0.0
        {
            simple = 1.0;
        }
        let iteration = if phase == Phase::After && self.iterations.is_infinite() {
            f64::INFINITY
        } else if simple == 1.0 {
            overall.floor() - 1.0
        } else {
            overall.floor()
        };
        let mut turn = iteration;
        if self.direction == Direction::AlternateReverse {
            turn += 1.0;
        }
        let forwards = match self.direction {
            Direction::Normal => true,
            Direction::Reverse => false,
            Direction::Alternate | Direction::AlternateReverse => {
                turn.is_infinite() || turn % 2.0 == 0.0
            }
        };
        Sample {
            progress: Some(if forwards { simple } else { 1.0 - simple }),
            running,
        }
    }
}

/// One property's value at one keyframe offset.
#[derive(Debug, Clone, PartialEq)]
struct PropertyKeyframe {
    offset: f64,
    value: String,
    /// The keyframe's own `animation-timing-function`, which eases the
    /// interval that starts at this keyframe.
    easing: Option<TimingFunction>,
}

/// A `@keyframes` rule compiled for sampling: for each animated longhand,
/// its keyframes sorted by offset with at most one per offset.
#[derive(Debug, Default, Clone, PartialEq)]
struct KeyframeSet {
    properties: Vec<(String, Vec<PropertyKeyframe>)>,
}

impl KeyframeSet {
    /// Compiles the keyframe rules of one `@keyframes` block.
    ///
    /// Keyframe rules that share an offset cascade: for each property the
    /// later declaration wins. `!important` declarations and `animation-*`
    /// properties other than `animation-timing-function` are ignored
    /// (CSS Animations 1 §3).
    fn compile(keyframes: &[crate::stylesheet::Keyframe]) -> Self {
        // Offset bits -> (easing, property -> value), in offset order.
        let mut frames: BTreeMap<u64, (Option<TimingFunction>, BTreeMap<String, String>)> =
            BTreeMap::new();
        for keyframe in keyframes {
            let declarations = compile_declarations(&keyframe.declarations);
            for offset in &keyframe.offsets {
                let offset = f64::from(*offset);
                // Offsets are non-negative, so their bit patterns sort in
                // numeric order.
                let (easing, values) = frames.entry(offset.to_bits()).or_default();
                for declaration in declarations.iter().filter(|d| !d.important) {
                    if declaration.property == "animation-timing-function" {
                        if let Some(timing) = TimingFunction::parse(&declaration.value) {
                            *easing = Some(timing);
                        }
                    } else if !declaration.property.starts_with("animation") {
                        values.insert(declaration.property.clone(), declaration.value.clone());
                    }
                }
            }
        }
        let mut properties: BTreeMap<String, Vec<PropertyKeyframe>> = BTreeMap::new();
        for (bits, (easing, values)) in frames {
            for (property, value) in values {
                properties
                    .entry(property)
                    .or_default()
                    .push(PropertyKeyframe {
                        offset: f64::from_bits(bits),
                        value,
                        easing,
                    });
            }
        }
        Self {
            properties: properties.into_iter().collect(),
        }
    }
}

/// Every `@keyframes` rule in the author stylesheets, by name.
#[derive(Debug, Default)]
pub(crate) struct KeyframesRegistry {
    sets: HashMap<String, KeyframeSet>,
}

impl KeyframesRegistry {
    /// Collects top-level `@keyframes` rules. When several share a name, the
    /// last in document order wins (CSS Animations 1 §3).
    pub(crate) fn new(stylesheets: &[Stylesheet]) -> Self {
        let mut sets = HashMap::new();
        for rule in stylesheets.iter().flat_map(|sheet| &sheet.at_rules) {
            if !matches!(
                rule.name.as_str(),
                "keyframes" | "-webkit-keyframes" | "-moz-keyframes"
            ) {
                continue;
            }
            match parse_keyframes_name(&rule.prelude) {
                Some(name) => {
                    log::trace!("@keyframes {name}: {} keyframe rules", rule.keyframes.len());
                    sets.insert(name, KeyframeSet::compile(&rule.keyframes));
                }
                None => log::debug!("ignoring @keyframes with invalid name {:?}", rule.prelude),
            }
        }
        Self { sets }
    }

    /// Whether no `@keyframes` rules were found.
    pub(crate) fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }
}

/// What the cascade knows about one element when applying its animations.
pub(crate) struct ElementContext<'a> {
    /// The parent's computed declarations, used for `inherit` in keyframes.
    pub(crate) parent: Option<&'a [Declaration]>,
    /// The parent's computed font size, in CSS pixels.
    pub(crate) parent_font_size: f32,
    /// The root element's computed font size, in CSS pixels.
    pub(crate) root_font_size: f32,
    /// Properties whose cascaded value came from an `!important`
    /// declaration; animations do not override them.
    pub(crate) important: &'a [&'a str],
}

/// Replaces the element's values in `properties` with its animations' values
/// `time` seconds after the timeline origin.
///
/// `properties` holds the element's cascaded values after CSS-wide keywords
/// are resolved, including inherited ones. Animations later in
/// `animation-name` win over earlier ones for the same property. Returns how
/// many of the element's animations can still change as time advances.
pub(crate) fn apply_animations(
    registry: &KeyframesRegistry,
    properties: &mut BTreeMap<String, String>,
    time: f64,
    element: &ElementContext<'_>,
) -> usize {
    if registry.is_empty() {
        return 0;
    }
    let mut running = 0;
    let mut effects = Vec::new();
    for animation in Animation::list(properties) {
        let Some(set) = registry.sets.get(&animation.name) else {
            log::trace!("animation-name {:?} has no @keyframes", animation.name);
            continue;
        };
        let sample = animation.sample(time);
        running += usize::from(sample.running);
        if let Some(progress) = sample.progress {
            effects.push((set, progress, animation.timing));
        }
    }
    if effects.is_empty() {
        return running;
    }

    // `font-size` first, as `em` in other properties resolves against it.
    let mut context = LengthContext {
        font_size: element.parent_font_size,
        root_font_size: element.root_font_size,
        viewport_width: 0.0,
        viewport_height: 0.0,
    };
    for font_size_pass in [true, false] {
        if !font_size_pass {
            context.font_size = properties
                .get("font-size")
                .and_then(|value| {
                    values::compute_font_size(
                        value,
                        element.parent_font_size,
                        element.root_font_size,
                    )
                })
                .unwrap_or(element.parent_font_size);
        }
        for (set, progress, timing) in &effects {
            for (property, keyframes) in &set.properties {
                if (property == "font-size") != font_size_pass
                    || element.important.contains(&property.as_str())
                {
                    continue;
                }
                let underlying = properties
                    .get(property)
                    .cloned()
                    .or_else(|| initial_value(property).map(str::to_owned));
                let value = sample_property(
                    property,
                    keyframes,
                    *progress,
                    underlying.as_deref(),
                    *timing,
                    element,
                    &context,
                );
                if let Some(value) = value {
                    log::trace!("animated {property}: {value}");
                    properties.insert(property.clone(), value);
                }
            }
        }
    }
    running
}

/// The initial value used where an animated property has no cascaded value,
/// or `None` if this engine does not know one.
fn initial_value(property: &str) -> Option<&'static str> {
    Some(match property {
        "width" | "height" | "top" | "right" | "bottom" | "left" | "flex-basis" => "auto",
        "opacity" | "flex-shrink" => "1",
        "flex-grow" | "order" => "0",
        // `normal` computes to zero in flex and grid containers.
        "gap" | "row-gap" | "column-gap" => "0px",
        "background-color" => "transparent",
        _ if property.starts_with("margin-")
            || property.starts_with("padding-")
            || (property.starts_with("border-") && property.ends_with("-radius")) =>
        {
            "0px"
        }
        _ => return None,
    })
}

/// Samples one property's keyframes at iteration `progress`
/// (Web Animations 1 §5.3.4, "The effect value of a keyframe effect").
///
/// Missing `0%` and `100%` keyframes take the `underlying` value. Returns
/// `None` if there is nothing to sample.
fn sample_property(
    property: &str,
    keyframes: &[PropertyKeyframe],
    progress: f64,
    underlying: Option<&str>,
    timing: TimingFunction,
    element: &ElementContext<'_>,
    context: &LengthContext,
) -> Option<String> {
    let mut frames: Vec<PropertyKeyframe> = Vec::with_capacity(keyframes.len() + 2);
    let first = keyframes.first()?;
    let last = keyframes.last()?;
    if first.offset > 0.0 {
        frames.push(PropertyKeyframe {
            offset: 0.0,
            value: underlying.unwrap_or(&first.value).to_owned(),
            easing: None,
        });
    }
    frames.extend(keyframes.iter().cloned());
    if last.offset < 1.0 {
        frames.push(PropertyKeyframe {
            offset: 1.0,
            value: underlying.unwrap_or(&last.value).to_owned(),
            easing: None,
        });
    }
    for frame in &mut frames {
        frame.value = resolve_keyword(property, &frame.value, element)?;
    }
    // The interval starts at the last keyframe at or before `progress` whose
    // offset is below 1, so `progress == 1` lands on the end of the last
    // interval.
    let start = frames
        .iter()
        .rposition(|frame| frame.offset <= progress && frame.offset < 1.0)
        .unwrap_or(0);
    let Some(end) = frames.get(start + 1) else {
        return Some(frames[start].value.clone());
    };
    let start = &frames[start];
    let span = end.offset - start.offset;
    let local = if span > 0.0 {
        (progress - start.offset) / span
    } else {
        1.0
    };
    let eased = start.easing.unwrap_or(timing).apply(local);
    Some(interpolate(
        property,
        &start.value,
        &end.value,
        eased,
        element,
        context,
    ))
}

/// Resolves CSS-wide keywords in a keyframe value; other values are returned
/// unchanged. Returns `None` when the value resolves to "no value".
fn resolve_keyword(property: &str, value: &str, element: &ElementContext<'_>) -> Option<String> {
    let keyword = value.trim().to_ascii_lowercase();
    if !CSS_WIDE_KEYWORDS.contains(&keyword.as_str()) {
        return Some(value.to_owned());
    }
    let parent = || {
        element.parent.and_then(|parent| {
            parent
                .iter()
                .find(|declaration| declaration.property == property)
                .map(|declaration| declaration.value.clone())
        })
    };
    let inherits = keyword == "inherit"
        || (keyword != "initial" && crate::cascade::is_inherited_property(property));
    if inherits && let Some(value) = parent() {
        return Some(value);
    }
    initial_value(property).map(str::to_owned)
}

/// Interpolates between two computed values at progress `t`, which may lie
/// outside `0..=1` for easing curves that overshoot.
fn interpolate(
    property: &str,
    from: &str,
    to: &str,
    t: f64,
    element: &ElementContext<'_>,
    context: &LengthContext,
) -> String {
    if t == 0.0 {
        return from.to_owned();
    }
    if t == 1.0 {
        return to.to_owned();
    }
    let interpolated = if property == "font-size" {
        let size = |value| {
            values::compute_font_size(value, element.parent_font_size, element.root_font_size)
        };
        size(from)
            .zip(size(to))
            .map(|(from, to)| values::format_px(lerp(from, to, t).max(0.0)))
    } else {
        interpolate_color(from, to, t).or_else(|| interpolate_components(from, to, t, context))
    };
    interpolated.unwrap_or_else(|| {
        // Discrete animation (Web Animations 1 §5.2): flip halfway.
        if t < 0.5 { from } else { to }.to_owned()
    })
}

fn lerp(from: f32, to: f32, t: f64) -> f32 {
    (f64::from(from) + (f64::from(to) - f64::from(from)) * t) as f32
}

/// Interpolates two colors in premultiplied sRGB (CSS Color 4 §12.3), so a
/// fade from `transparent` keeps the target hue.
fn interpolate_color(from: &str, to: &str, t: f64) -> Option<String> {
    let color = |text: &str| {
        if text.trim().eq_ignore_ascii_case("transparent") {
            Some(CssColor {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            })
        } else {
            parse_color(text)
        }
    };
    let (from, to) = (color(from)?, color(to)?);
    let alpha = |color: CssColor| f64::from(color.a) / 255.0;
    let (from_alpha, to_alpha) = (alpha(from), alpha(to));
    let out_alpha = (from_alpha + (to_alpha - from_alpha) * t).clamp(0.0, 1.0);
    let channel = |a: u8, b: u8| {
        let premultiplied_a = f64::from(a) * from_alpha;
        let premultiplied_b = f64::from(b) * to_alpha;
        let premultiplied = premultiplied_a + (premultiplied_b - premultiplied_a) * t;
        if out_alpha > 0.0 {
            (premultiplied / out_alpha).round().clamp(0.0, 255.0) as u8
        } else {
            0
        }
    };
    let (r, g, b) = (
        channel(from.r, to.r),
        channel(from.g, to.g),
        channel(from.b, to.b),
    );
    Some(if out_alpha >= 1.0 {
        format!("rgb({r}, {g}, {b})")
    } else {
        format!("rgba({r}, {g}, {b}, {})", format_number(out_alpha as f32))
    })
}

/// Interpolates space-separated numbers, percentages, and lengths component
/// by component. Identical components, such as matching keywords, are kept.
/// Returns `None` when any pair of components cannot be interpolated.
fn interpolate_components(from: &str, to: &str, t: f64, context: &LengthContext) -> Option<String> {
    let from_parts = split_components(from);
    let to_parts = split_components(to);
    if from_parts.len() != to_parts.len() || from_parts.is_empty() {
        return None;
    }
    let parts = from_parts
        .iter()
        .zip(&to_parts)
        .map(|(from, to)| interpolate_component(from, to, t, context))
        .collect::<Option<Vec<_>>>()?;
    Some(parts.join(" "))
}

fn interpolate_component(from: &str, to: &str, t: f64, context: &LengthContext) -> Option<String> {
    if let (Some(a), Some(b)) = (parse_number(from), parse_number(to)) {
        return Some(format_number(lerp(a, b, t)));
    }
    if let (Some(a), Some(b)) = (parse_percentage(from), parse_percentage(to)) {
        return Some(format!("{}%", format_number(lerp(a, b, t))));
    }
    if let (Some(a), Some(b)) = (parse_length(from), parse_length(to)) {
        if a.unit == b.unit {
            // Unitless zero parses as `px`, so take the unit from either side.
            let unit = [unit_of(from), unit_of(to)]
                .into_iter()
                .find(|unit| !unit.is_empty())
                .unwrap_or("px");
            return Some(format!(
                "{}{unit}",
                format_number(lerp(a.value, b.value, t))
            ));
        }
        if a.is_viewport_relative() || b.is_viewport_relative() {
            return None;
        }
        return Some(values::format_px(lerp(
            a.to_px(context),
            b.to_px(context),
            t,
        )));
    }
    from.eq_ignore_ascii_case(to).then(|| from.to_owned())
}

/// Returns the unit text after a dimension's number, such as `em` in `1.5em`.
fn unit_of(text: &str) -> &str {
    text.trim()
        .trim_start_matches(|c: char| c.is_ascii_digit() || matches!(c, '+' | '-' | '.'))
}

/// Formats a number with at most four decimal places and no trailing zeros.
fn format_number(value: f32) -> String {
    let rounded = (value * 10_000.0).round() / 10_000.0;
    // Avoid `-0`.
    format!("{}", if rounded == 0.0 { 0.0 } else { rounded })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animation(properties: &[(&str, &str)]) -> Vec<Animation> {
        let map = properties
            .iter()
            .map(|(property, value)| ((*property).to_owned(), (*value).to_owned()))
            .collect();
        Animation::list(&map)
    }

    fn one(properties: &[(&str, &str)]) -> Animation {
        animation(properties).remove(0)
    }

    #[test]
    fn shorthand_assigns_times_keywords_and_name() {
        let expanded =
            expand_shorthand("slide 2s ease-in 500ms infinite alternate both paused, fade 1s")
                .expect("valid shorthand");
        let value = |property: &str| {
            expanded
                .iter()
                .find(|declaration| declaration.property == property)
                .map(|declaration| declaration.value.as_str())
        };
        assert_eq!(value("animation-name"), Some("slide, fade"));
        assert_eq!(value("animation-duration"), Some("2s, 1s"));
        assert_eq!(value("animation-timing-function"), Some("ease-in, ease"));
        assert_eq!(value("animation-delay"), Some("500ms, 0s"));
        assert_eq!(value("animation-iteration-count"), Some("infinite, 1"));
        assert_eq!(value("animation-direction"), Some("alternate, normal"));
        assert_eq!(value("animation-fill-mode"), Some("both, none"));
        assert_eq!(value("animation-play-state"), Some("paused, running"));

        // `none` fills the fill-mode slot before the name slot.
        let expanded = expand_shorthand("none none 1s").expect("valid shorthand");
        assert_eq!(expanded[0].value, "none");
        assert_eq!(expanded[6].value, "none");
        assert!(expand_shorthand("a 1s 2s 3s").is_none());
        assert!(expand_shorthand("1px").is_none());
        assert!(
            expand_shorthand("inherit")
                .unwrap()
                .iter()
                .all(|declaration| declaration.value == "inherit")
        );
    }

    #[test]
    fn cubic_bezier_easing_matches_known_points() {
        let ease = TimingFunction::parse("ease").unwrap();
        assert_eq!(ease.apply(0.0), 0.0);
        assert_eq!(ease.apply(1.0), 1.0);
        // Chrome's `ease` at 50% input is about 0.8024.
        assert!((ease.apply(0.5) - 0.8024).abs() < 0.001);
        let linear_bezier = TimingFunction::parse("cubic-bezier(0, 0, 1, 1)").unwrap();
        assert!((linear_bezier.apply(0.3) - 0.3).abs() < 1e-5);
        assert!(TimingFunction::parse("cubic-bezier(2, 0, 1, 1)").is_none());
        assert!(TimingFunction::parse("linear(0, 1)").is_none());
    }

    #[test]
    fn steps_easing_jumps_at_the_requested_positions() {
        let end = TimingFunction::parse("steps(4)").unwrap();
        assert_eq!(end.apply(0.0), 0.0);
        assert_eq!(end.apply(0.3), 0.25);
        assert_eq!(end.apply(1.0), 1.0);
        let start = TimingFunction::parse("step-start").unwrap();
        assert_eq!(start.apply(0.1), 1.0);
        let none = TimingFunction::parse("steps(3, jump-none)").unwrap();
        assert_eq!(none.apply(0.5), 0.5);
        assert!(TimingFunction::parse("steps(1, jump-none)").is_none());
        assert!(TimingFunction::parse("steps(0)").is_none());
    }

    #[test]
    fn lists_repeat_to_match_animation_names() {
        let animations = animation(&[
            ("animation-name", "a, none, b, c"),
            ("animation-duration", "1s, 2s"),
            ("animation-delay", "bogus"),
        ]);
        let names: Vec<_> = animations.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "c"]);
        let durations: Vec<_> = animations.iter().map(|a| a.duration).collect();
        assert_eq!(durations, [1.0, 1.0, 2.0]);
        assert!(animations.iter().all(|a| a.delay == 0.0));
    }

    #[test]
    fn sampling_follows_delay_fill_and_iterations() {
        let base = [
            ("animation-name", "x"),
            ("animation-duration", "2s"),
            ("animation-delay", "1s"),
        ];
        let plain = one(&base);
        assert_eq!(plain.sample(0.5).progress, None);
        assert!(plain.sample(0.5).running);
        assert_eq!(plain.sample(2.0).progress, Some(0.5));
        assert_eq!(plain.sample(3.0).progress, None);
        assert!(!plain.sample(3.0).running);

        let mut filled = base.to_vec();
        filled.push(("animation-fill-mode", "both"));
        let filled = one(&filled);
        assert_eq!(filled.sample(0.0).progress, Some(0.0));
        assert_eq!(filled.sample(10.0).progress, Some(1.0));

        let fractional = one(&[
            ("animation-name", "x"),
            ("animation-duration", "1s"),
            ("animation-iteration-count", "1.5"),
            ("animation-fill-mode", "forwards"),
        ]);
        assert_eq!(fractional.sample(5.0).progress, Some(0.5));

        let negative_delay = one(&[
            ("animation-name", "x"),
            ("animation-duration", "4s"),
            ("animation-delay", "-1s"),
        ]);
        assert_eq!(negative_delay.sample(0.0).progress, Some(0.25));
    }

    #[test]
    fn directions_reverse_and_alternate_iterations() {
        let sample = |direction: &str, time: f64| {
            one(&[
                ("animation-name", "x"),
                ("animation-duration", "1s"),
                ("animation-iteration-count", "infinite"),
                ("animation-direction", direction),
            ])
            .sample(time)
            .progress
            .unwrap()
        };
        assert_eq!(sample("normal", 1.25), 0.25);
        assert_eq!(sample("reverse", 1.25), 0.75);
        assert_eq!(sample("alternate", 0.25), 0.25);
        assert_eq!(sample("alternate", 1.25), 0.75);
        assert_eq!(sample("alternate-reverse", 0.25), 0.75);
    }

    #[test]
    fn paused_animations_hold_their_first_frame() {
        let paused = one(&[
            ("animation-name", "x"),
            ("animation-duration", "1s"),
            ("animation-play-state", "paused"),
        ]);
        let sample = paused.sample(0.5);
        assert_eq!(sample.progress, Some(0.0));
        assert!(!sample.running);
    }

    #[test]
    fn values_interpolate_by_type() {
        let element = ElementContext {
            parent: None,
            parent_font_size: 16.0,
            root_font_size: 16.0,
            important: &[],
        };
        let context = LengthContext {
            font_size: 10.0,
            root_font_size: 16.0,
            viewport_width: 0.0,
            viewport_height: 0.0,
        };
        let at = |property, from, to, t| interpolate(property, from, to, t, &element, &context);
        assert_eq!(at("width", "0", "100px", 0.25), "25px");
        assert_eq!(at("width", "1em", "30px", 0.5), "20px");
        assert_eq!(at("width", "10%", "30%", 0.5), "20%");
        assert_eq!(at("opacity", "0", "1", 0.5), "0.5");
        assert_eq!(at("color", "red", "blue", 0.5), "rgb(128, 0, 128)");
        assert_eq!(
            at("background-color", "transparent", "red", 0.5),
            "rgba(255, 0, 0, 0.5)"
        );
        assert_eq!(
            at("border-top-left-radius", "0 4px", "10px 8px", 0.5),
            "5px 6px"
        );
        assert_eq!(at("font-size", "16px", "2em", 0.5), "24px");
        // Values without a shared numeric type flip at the midpoint.
        assert_eq!(at("width", "auto", "100px", 0.4), "auto");
        assert_eq!(at("width", "auto", "100px", 0.5), "100px");
        assert_eq!(at("width", "10vw", "100px", 0.6), "100px");
    }
}
