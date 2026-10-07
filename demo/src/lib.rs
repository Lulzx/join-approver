//! Join Approver: a 26 s demo cut to an original 90 BPM boom-bap beat.
//!
//! Every scene starts on a bar line of `media/beat.wav` (BAR = 8/3 s, made by
//! `scripts/beat.py`); bar 1 is the quiet intro and the drums drop on bar 2,
//! where the title lands. The backdrop is three GPU shaders ported from
//! Shader Effects Inc. (MIT): FlowingGradient, Godrays and FilmGrain, drawn
//! once at the video level so they flow continuously under the scene cuts.
//! App screens are real UI renders with invented data (`scripts/screens.ts`).
use fframes::{
    AnimateRuntimeInput, AudioMap, AudioTimestamp::*, AudioTrack, Color, Duration,
    FFramesContext, FontQuery, Frame, Scene, Scenes, Shader, ShaderUniforms, Svgr, Video,
    animation::{AnimationRuntime, Easing},
    include_media_dir,
};
use std::sync::LazyLock;

include_media_dir!(pub struct JoinApproverDemoMedia, "media");

pub const WIDTH: usize = 1920;
pub const HEIGHT: usize = 1080;
const FPS: usize = 30;

// ---- the beat ----
const BPM: f32 = 90.0;
const BEAT: f32 = 60.0 / BPM;
const BAR: f32 = 4.0 * BEAT;
const STEP: f32 = BEAT / 4.0;
const SWING: f32 = 0.58;
const DROP: f32 = BAR; // drums start on bar 2

// Scene starts, in bars: the cuts land on the downbeat.
const S_PROBLEM: f32 = 2.0 * BAR;
const S_APP: f32 = 3.0 * BAR;
const S_RUN: f32 = 5.0 * BAR;
const S_PROOF: f32 = 7.0 * BAR;
const S_OUTRO: f32 = 8.0 * BAR;
const END: f32 = S_OUTRO + 5.0;

/// fframes keys fonts by family name, so each Inter weight in `media/` was
/// renamed to a family of its own; a weight is picked by family.
fn font(weight: u16) -> &'static str {
    match weight {
        850.. => "Inter Black",
        750.. => "Inter ExtraBold",
        550.. => "Inter SemiBold",
        _ => "Inter Medium",
    }
}
const WHITE: &str = "#ffffff";
const SOFT: &str = "#c9c6e4";
const GREEN: &str = "#32d74b";
const BLUE: &str = "#2aabee";

/// Kick times, exactly as `beat.py` places them: drives the pulse.
static KICKS: LazyLock<Vec<f32>> = LazyLock::new(|| {
    let mut out = Vec::new();
    for bar in 1..9 {
        let steps: &[usize] = if bar % 4 == 3 { &[0, 3, 10, 13] } else { &[0, 7, 10] };
        for &s in steps {
            out.push(step_time(bar, s));
        }
    }
    out.push(step_time(9, 0));
    out
});

/// Scene length in whole frames, rounded from absolute times so cuts never
/// drift off the bar lines.
fn frames(start: f32, end: f32) -> Duration<'static> {
    let f = |t: f32| (t * FPS as f32).round() as usize;
    Duration::Frames(f(end) - f(start))
}

fn step_time(bar: usize, step: usize) -> f32 {
    let mut t = bar as f32 * BAR + step as f32 * STEP;
    if step % 2 == 1 {
        t += (SWING - 0.5) * 2.0 * STEP;
    }
    t
}

/// 0..1 spike on every kick, decaying over ~150 ms.
fn kick_pulse(t: f32) -> f32 {
    KICKS
        .iter()
        .filter(|&&k| t >= k && t - k < 0.6)
        .map(|&k| (-(t - k) * 14.0).exp())
        .fold(0.0, f32::max)
}

// ---- easing helpers ----
const EXPO: Easing = Easing::CubicBezier(0.16, 1.0, 0.3, 1.0);
const IN_OUT: Easing = Easing::CubicBezier(0.65, 0.0, 0.35, 1.0);
const SNAPPY: Easing = Easing::Spring { mass: 1.0, stiffness: 300.0, damping: 24.0 };
const SOFT_SPRING: Easing = Easing::Spring { mass: 1.0, stiffness: 160.0, damping: 18.0 };

fn tween(frame: &Frame, start: f32, dur: f32, from: f32, to: f32, e: &Easing) -> f32 {
    frame.animate_runtime(AnimateRuntimeInput {
        on_second: start,
        from,
        to,
        animation_runtime: &AnimationRuntime::new(dur, e),
    })
}

/// 0 -> 1 over `dur` from `start`, ease-out expo.
fn appear(frame: &Frame, start: f32, dur: f32) -> f32 {
    tween(frame, start, dur, 0.0, 1.0, &EXPO)
}

/// 1 -> 0 over the last `dur` seconds of a scene of `len` seconds.
fn exit(frame: &Frame, len: f32, dur: f32) -> f32 {
    tween(frame, len - dur, dur, 1.0, 0.0, &Easing::EaseIn)
}

fn spring_in(frame: &Frame, start: f32, from: f32) -> f32 {
    tween(frame, start, 3.0, from, 0.0, &SOFT_SPRING)
}

fn thousands(n: f32) -> String {
    let n = n.round().max(0.0) as u64;
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

// ---- color: the gradient's palette moves between scenes in OKLab ----
fn srgb_to_oklab(hex: &str) -> [f32; 3] {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    let lin = |c: u32| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let (r, g, b) = (lin((v >> 16) & 255), lin((v >> 8) & 255), lin(v & 255));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

type Palette = [[f32; 3]; 4];

fn palette(hexes: [&str; 4]) -> Palette {
    hexes.map(srgb_to_oklab)
}

/// (time the palette is fully in, palette)
static PALETTES: LazyLock<Vec<(f32, Palette)>> = LazyLock::new(|| {
    vec![
        (0.0, palette(["#05021a", "#3b1488", "#a8305a", "#b8502c"])),
        (S_APP, palette(["#03050f", "#1e2a78", "#2aabee", "#6d28d9"])),
        (S_PROOF, palette(["#02100b", "#065f46", "#22c55e", "#2aabee"])),
        (S_OUTRO, palette(["#06010f", "#4c13b0", "#b8325a", "#b8741a"])),
    ]
});

fn palette_at(t: f32) -> Palette {
    let p = &*PALETTES;
    let mut cur = p[0].1;
    for w in p.windows(2) {
        let (t1, b) = w[1];
        let from = t1 - 0.9; // blend over the last 0.9 s before the cut
        if t >= t1 {
            cur = b;
        } else if t > from {
            let k = (t - from) / 0.9;
            let k = k * k * (3.0 - 2.0 * k);
            let a = cur;
            let mut out = a;
            for i in 0..4 {
                for c in 0..3 {
                    out[i][c] = a[i][c] + (b[i][c] - a[i][c]) * k;
                }
            }
            return out;
        } else {
            return cur;
        }
    }
    cur
}

// ---- the video ----
pub struct JoinApproverDemoVideo<'a> {
    pub media: &'a JoinApproverDemoMedia,
    gradient: Shader,
    rays: Shader,
    grain: Shader,
    /// Film grain strength. Noise doesn't compress, so the web cut sets
    /// GRAIN=0 for a much smaller file.
    grain_strength: f32,
}

impl<'a> JoinApproverDemoVideo<'a> {
    pub fn new(media: &'a JoinApproverDemoMedia, _title: &'a str) -> Self {
        Self {
            media,
            gradient: Shader::sksl(include_str!("shaders/flowing_gradient.sksl")),
            rays: Shader::sksl(include_str!("shaders/godrays.sksl")),
            grain: Shader::sksl(include_str!("shaders/film_grain.sksl")),
            grain_strength: std::env::var("GRAIN").ok().and_then(|g| g.parse().ok()).unwrap_or(0.07),
        }
    }
}

impl std::fmt::Debug for JoinApproverDemoVideo<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinApproverDemoVideo").finish()
    }
}

/// Where the godrays shine from, their color and strength, by time.
fn rays_at(t: f32) -> (f32, f32, [f32; 3], f32) {
    let bump = |start: f32, end: f32, peak: f32| {
        let up = ((t - start) / 0.5).clamp(0.0, 1.0);
        let down = ((end - t) / 0.6).clamp(0.0, 1.0);
        (up.min(down)) * peak
    };
    if t < S_PROBLEM {
        // Faint in the quiet bar, full on the drop.
        let pre = bump(0.4, S_PROBLEM, 0.35);
        let drop = if t >= DROP { bump(DROP, S_PROBLEM, 1.0) } else { 0.0 };
        (0.5, 0.36, [0.65, 0.55, 1.0], pre.max(drop))
    } else if t >= S_PROOF && t < S_OUTRO {
        (0.5, 0.2, [0.35, 1.0, 0.55], bump(S_PROOF, S_OUTRO, 0.8))
    } else if t >= S_OUTRO {
        (0.5, 0.27, [1.0, 0.8, 0.6], bump(S_OUTRO, END, 0.6))
    } else {
        (0.5, 0.5, [1.0, 1.0, 1.0], 0.0)
    }
}

/// How far the backdrop is dimmed so app screens read clearly.
fn dim_at(t: f32) -> f32 {
    let ramp = |a: f32, b: f32| ((t - a) / (b - a)).clamp(0.0, 1.0);
    if t < S_APP - 0.4 {
        0.18
    } else if t < S_PROOF {
        0.5 * ramp(S_APP - 0.4, S_APP + 0.3)
    } else if t < S_OUTRO {
        0.5 - 0.15 * ramp(S_PROOF, S_PROOF + 0.4)
    } else {
        0.35 - 0.12 * ramp(S_OUTRO - 0.2, S_OUTRO + 0.6)
    }
}

impl Video for JoinApproverDemoVideo<'_> {
    const FPS: usize = FPS;
    const WIDTH: usize = WIDTH;
    const HEIGHT: usize = HEIGHT;
    const BACKGROUND_COLOR: Color = Color::BLACK;

    fn duration(&self) -> Duration<'_> {
        Duration::Auto
    }

    fn audio(&self) -> AudioMap<'_> {
        AudioMap::from([AudioTrack::new("beat.wav", Second(0.)..Eof).gain_db(-5.)])
    }

    fn define_scenes(&self) -> Scenes<'_> {
        Scenes::from(vec![
            &Intro as &dyn Scene,
            &Problem,
            &App,
            &Approving,
            &Proof,
            &Outro,
        ])
    }

    fn render_frame<'a>(&'a self, frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.global_index as f32 / FPS as f32;
        let pal = palette_at(t);
        let pulse = kick_pulse(t);
        let gradient = self.gradient.draw(
            &frame,
            ShaderUniforms::new()
                .float("uSpeed", 1.6)
                .float("uDistortion", 0.5)
                .float("uSeed", 3.0)
                .float3("uLabA", pal[0][0], pal[0][1], pal[0][2])
                .float3("uLabB", pal[1][0], pal[1][1], pal[1][2])
                .float3("uLabC", pal[2][0], pal[2][1], pal[2][2])
                .float3("uLabD", pal[3][0], pal[3][1], pal[3][2]),
        );
        let (cx, cy, rc, ray_fade) = rays_at(t);
        let rays = self.rays.draw(
            &frame,
            ShaderUniforms::new()
                .float2("uCenter", cx, cy)
                .float4("uRayColor", rc[0], rc[1], rc[2], 0.9)
                .float("uDensity", 0.35)
                .float("uIntensity", 0.8)
                .float("uSpotty", 0.9)
                .float("uSpeed", 1.0)
                .float("uFade", ray_fade * (0.85 + 0.15 * pulse)),
        );
        let grain = (self.grain_strength > 0.0)
            .then(|| self.grain.draw(&frame, ShaderUniforms::new().float("uStrength", self.grain_strength)).href());

        // The drop: a white flash, and a breath of light on every kick.
        let flash = if t >= DROP { 0.45 * (-(t - DROP) * 7.0).exp() } else { 0.0 };
        let dim = dim_at(t);
        let fade_out = ((END - t) / 0.8).clamp(0.0, 1.0);
        let fade_in = (t / 0.4).clamp(0.0, 1.0);
        let black = 1.0 - fade_out.min(fade_in);

        fframes::svgr!(
            <svg xmlns="http://www.w3.org/2000/svg" width={WIDTH} height={HEIGHT} viewBox="0 0 1920 1080">
                <defs>
                    <radialGradient id="vignette" cx="0.5" cy="0.5" r="0.75">
                        <stop offset="0.55" stop-color="#000" stop-opacity="0" />
                        <stop offset="1" stop-color="#000" stop-opacity="0.7" />
                    </radialGradient>
                </defs>
                <image href={gradient.href()} x="0" y="0" width="1920" height="1080" />
                <rect width="1920" height="1080" fill="#ffffff" opacity={0.035 * pulse} />
                <rect width="1920" height="1080" fill="#000000" opacity={dim} />
                <image href={rays.href()} x="0" y="0" width="1920" height="1080" />
                <rect width="1920" height="1080" fill="url(#vignette)" />
                {ctx.render_scenes(&frame)}
                <rect width="1920" height="1080" fill="#ffffff" opacity={flash} />
                {match grain {
                    Some(href) => fframes::svgr!(<image href={href} x="0" y="0" width="1920" height="1080" />),
                    None => Svgr::empty(),
                }}
                <rect width="1920" height="1080" fill="#000000" opacity={black} />
            </svg>
        )
    }
}

// ---- shared pieces ----

/// A macOS window around an app screen, drawn `w` wide with its top-left at
/// (x, y). The screen is a 2x render of a 1100x700 window.
fn window<'a>(ctx: &FFramesContext<'a, '_>, file: &str, id: &'a str, x: f32, y: f32, w: f32) -> Svgr<'a> {
    let Some(img) = ctx.get_image(file) else { return Svgr::empty() };
    let h = w * 700.0 / 1100.0;
    let s = w / 1100.0;
    let r = 12.0 * s;
    fframes::svgr!(
        <g>
            <defs>
                <clipPath id={id}>
                    <rect x={x} y={y} width={w} height={h} rx={r} />
                </clipPath>
                <filter id="winshadow" x="-20%" y="-20%" width="140%" height="150%">
                    <feGaussianBlur stdDeviation="28" />
                </filter>
            </defs>
            <rect x={x + 10.0} y={y + 34.0} width={w - 20.0} height={h} rx={r} fill="#000" opacity="0.6" filter="url(#winshadow)" />
            <g clip-path={format!("url(#{id})")}>
                <image href={img.href()} x={x} y={y} width={w} height={h} />
            </g>
            <rect x={x} y={y} width={w} height={h} rx={r} fill="none" stroke="#ffffff" stroke-opacity="0.16" stroke-width="1.5" />
            <circle cx={x + 20.0 * s} cy={y + 22.0 * s} r={6.0 * s} fill="#ff5f57" />
            <circle cx={x + 40.0 * s} cy={y + 22.0 * s} r={6.0 * s} fill="#febc2e" />
            <circle cx={x + 60.0 * s} cy={y + 22.0 * s} r={6.0 * s} fill="#28c840" />
        </g>
    )
}

/// A pill-shaped callout: a colored dot and a line of text.
fn callout<'a>(x: f32, y: f32, text: &'a str, color: &'a str, width: f32) -> Svgr<'a> {
    fframes::svgr!(
        <g>
            <rect x={x} y={y} width={width} height="76" rx="38" fill="#0c0b16" fill-opacity="0.86" stroke="#ffffff" stroke-opacity="0.14" />
            <circle cx={x + 40.0} cy={y + 38.0} r="11" fill={color} />
            <text x={x + 66.0} y={y + 49.0} font-family={font(600)} font-weight="600" font-size="31" fill={WHITE}>{text}</text>
        </g>
    )
}

/// A callout that pops in on a beat (spring scale from 0.85) and leaves at `out`.
fn pop<'a>(frame: &Frame, at: f32, out: f32, x: f32, y: f32, text: &'a str, color: &'a str, width: f32) -> Svgr<'a> {
    let o = appear(frame, at, 0.25) * (1.0 - appear(frame, out, 0.2));
    if o <= 0.001 {
        return Svgr::empty();
    }
    let s = 1.0 - tween(frame, at, 3.0, 0.15, 0.0, &SNAPPY);
    let (px, py) = (x + width / 2.0, y + 38.0);
    let transform = format!("translate({} {}) scale({s}) translate({} {})", px, py, -px, -py);
    fframes::svgr!(<g opacity={o} transform={transform}>{callout(x, y, text, color, width)}</g>)
}

/// Zoom (scale `z`) toward a focus point (fx, fy), keeping it at the same
/// place on screen: a camera push.
fn camera(z: f32, fx: f32, fy: f32, dx: f32, dy: f32) -> String {
    format!("translate({} {}) scale({z}) translate({} {})", fx + dx, fy + dy, -fx, -fy)
}

/// Big headline used across scenes.
fn headline<'a>(x: f32, y: f32, size: f32, weight: u16, color: &'a str, anchor: &'a str, text: &'a str) -> Svgr<'a> {
    let spacing = -(size * 0.035);
    fframes::svgr!(
        <text x={x} y={y} font-family={font(weight)} font-weight={weight} font-size={size} letter-spacing={spacing} text-anchor={anchor} fill={color}>{text}</text>
    )
}

// ---- scenes ----

#[derive(Debug)]
struct Intro;

impl Scene for Intro {
    fn duration(&self) -> Duration<'_> {
        frames(0.0, S_PROBLEM)
    }

    fn render_frame<'a>(&'a self, mut frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.seconds();
        let len = S_PROBLEM;
        let out = exit(&frame, len, 0.22);

        // Bar 1: the icon breathes in. The drop: it rises and the title lands.
        let icon_o = appear(&frame, 0.35, 1.4);
        let icon_s = 0.82 + 0.18 * appear(&frame, 0.35, 2.0) + 0.04 * kick_pulse(t);
        let icon_y = 360.0 - 120.0 * tween(&frame, DROP, 0.7, 0.0, 1.0, &EXPO);
        let icon = ctx
            .get_image("app-icon.png")
            .map(|img| {
                let size = 300.0 * icon_s;
                fframes::svgr!(<image href={img.href()} x={960.0 - size / 2.0} y={icon_y + 150.0 - size / 2.0} width={size} height={size} opacity={icon_o} />)
            })
            .unwrap_or_default();
        let glow_o = 0.55 * icon_o * (0.7 + 0.3 * kick_pulse(t));

        let eyebrow_o = appear(&frame, 1.1, 0.8) * (1.0 - appear(&frame, DROP - 0.25, 0.25));

        // Title: each word springs up on the drop, 90 ms apart.
        // Measured with the real font: the pair is centred as one line.
        let q = FontQuery { family: font(900), size: 168, weight: 900, ..Default::default() };
        let w_join = frame.text_width(ctx, q, "Join").unwrap_or(380) as f32;
        let w_appr = frame.text_width(ctx, q, "Approver").unwrap_or(780) as f32;
        let gap = 46.0;
        let left = 960.0 - (w_join + gap + w_appr) / 2.0;
        let words = [("Join", left + w_join / 2.0), ("Approver", left + w_join + gap + w_appr / 2.0)];
        let title: Vec<Svgr> = words
            .iter()
            .enumerate()
            .map(|(i, (w, x))| {
                let at = DROP + i as f32 * 0.09;
                let o = appear(&frame, at, 0.18);
                let dy = spring_in(&frame, at, 90.0);
                let s = 1.0 + tween(&frame, at, 3.0, 0.25, 0.0, &SNAPPY);
                let transform = format!("translate({x} {}) scale({s}) translate({} 0)", 700.0 + dy, -x);
                fframes::svgr!(<g opacity={o} transform={transform}>{headline(*x, 0.0, 168.0, 900, WHITE, "middle", w)}</g>)
            })
            .collect();

        let line1 = appear(&frame, DROP + 0.75, 0.6);
        let line2 = appear(&frame, DROP + 1.25, 0.6);

        fframes::svgr!(
            <g opacity={out}>
                <defs>
                    <radialGradient id="iconglow">
                        <stop offset="0" stop-color="#8b7bff" stop-opacity="0.9" />
                        <stop offset="1" stop-color="#8b7bff" stop-opacity="0" />
                    </radialGradient>
                </defs>
                <circle cx="960" cy={icon_y + 150.0} r="330" fill="url(#iconglow)" opacity={glow_o} />
                {icon}
                <g opacity={eyebrow_o}>
                    {headline(960.0, 720.0, 40.0, 600, SOFT, "middle", "for Telegram channel admins")}
                </g>
                {title}
                <g opacity={line1} transform={format!("translate(0 {})", spring_in(&frame, DROP + 0.75, 30.0))}>
                    {headline(960.0, 815.0, 54.0, 600, WHITE, "middle", "Approve every join request.")}
                </g>
                <g opacity={line2} transform={format!("translate(0 {})", spring_in(&frame, DROP + 1.25, 30.0))}>
                    {headline(960.0, 892.0, 54.0, 800, GREEN, "middle", "Prove every one landed.")}
                </g>
            </g>
        )
    }
}

#[derive(Debug)]
struct Problem;

impl Scene for Problem {
    fn duration(&self) -> Duration<'_> {
        frames(S_PROBLEM, S_APP)
    }

    fn render_frame<'a>(&'a self, frame: Frame, _ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.seconds();
        let len = S_APP - S_PROBLEM;
        let out = exit(&frame, len, 0.22);
        let count = tween(&frame, 0.05, 1.5, 0.0, 1186.0, &EXPO);
        let num_o = appear(&frame, 0.0, 0.2);
        let num_s = 1.0 + tween(&frame, 0.0, 3.0, 0.2, 0.0, &SNAPPY) + 0.015 * kick_pulse(t + S_PROBLEM);
        let sub_o = appear(&frame, 0.55, 0.5);
        let q_o = appear(&frame, 4.0 * BEAT - 0.9, 0.35);
        fframes::svgr!(
            <g opacity={out}>
                <g opacity={num_o} transform={format!("translate(960 520) scale({num_s}) translate(-960 -520)")}>
                    {headline(960.0, 560.0, 290.0, 900, WHITE, "middle", Box::leak(thousands(count).into_boxed_str()))}
                </g>
                <g opacity={sub_o} transform={format!("translate(0 {})", spring_in(&frame, 0.55, 30.0))}>
                    {headline(960.0, 668.0, 58.0, 600, SOFT, "middle", "people waiting to join your channel")}
                </g>
                <g opacity={q_o}>
                    {headline(960.0, 790.0, 48.0, 700, WHITE, "middle", "Approving them one by one, by hand?")}
                </g>
            </g>
        )
    }
}

#[derive(Debug)]
struct App;

impl Scene for App {
    fn duration(&self) -> Duration<'_> {
        frames(S_APP, S_RUN)
    }

    fn render_frame<'a>(&'a self, frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let len = S_RUN - S_APP;
        let out = exit(&frame, len, 0.22);
        // The window rises in on the downbeat...
        let rise = spring_in(&frame, 0.0, 260.0);
        let o = appear(&frame, 0.0, 0.35);
        // ...then the camera pushes toward the stats on bar 2.
        let z = 1.0 + 0.32 * tween(&frame, BAR, 1.0, 0.0, 1.0, &IN_OUT);
        let pan = -120.0 * tween(&frame, BAR, 1.0, 0.0, 1.0, &IN_OUT);
        let (x, y, w) = (240.0, 150.0, 1440.0);
        // A step into the confirm sheet on the last beat pair.
        let confirm_o = appear(&frame, BAR + 2.0 * BEAT, 0.3);

        let cam = camera(z, 1180.0, 330.0, pan * 0.4, 30.0 + rise);
        let callouts = vec![
            pop(&frame, BEAT * 1.0, BAR - 0.1, 70.0, 300.0, "Only channels you own or run", BLUE, 540.0),
            pop(&frame, BEAT * 2.0, BAR - 0.1, 70.0, 400.0, "Search everything, ⌘F", BLUE, 420.0),
            pop(&frame, BAR + BEAT * 0.5, BAR + 2.0 * BEAT, 1150.0, 830.0, "Live counts, cached locally", GREEN, 520.0),
            pop(&frame, BAR + 2.0 * BEAT + 0.15, len, 1190.0, 830.0, "One click. Confirmed.", GREEN, 400.0),
        ];
        fframes::svgr!(
            <g opacity={out}>
                <g opacity={o} transform={cam}>
                    {window(ctx, "app-channel.png", "w-app", x, y, w)}
                    <g opacity={confirm_o}>{window(ctx, "app-confirm.png", "w-confirm", x, y, w)}</g>
                </g>
                {callouts}
            </g>
        )
    }
}

#[derive(Debug)]
struct Approving;

impl Scene for Approving {
    fn duration(&self) -> Duration<'_> {
        frames(S_RUN, S_PROOF)
    }

    fn render_frame<'a>(&'a self, frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.seconds();
        let len = S_PROOF - S_RUN;
        let out = exit(&frame, len, 0.22);
        let o = appear(&frame, 0.0, 0.25);
        // Push in from wide to the live stats, then drift down the feed.
        let z = 1.04 + 0.36 * tween(&frame, 0.0, 1.6, 0.0, 1.0, &IN_OUT) + 0.004 * kick_pulse(t + S_RUN);
        let fy = 420.0 + 170.0 * tween(&frame, BAR, 2.0, 0.0, 1.0, &IN_OUT);
        let cam = camera(z, 1230.0, fy, -40.0, 60.0 - 130.0 * tween(&frame, BAR, 2.0, 0.0, 1.0, &IN_OUT));

        // A ticker of approvals, one per beat: the rhythm of the run.
        let approved = 612.0 + (t / BEAT).floor().max(0.0);
        let tick = (t % BEAT) / BEAT;
        let tick_s = 1.0 + 0.08 * (-(tick * 8.0)).exp();
        let ticker_o = appear(&frame, 0.3, 0.3);

        let callouts = vec![
            pop(&frame, BEAT, BAR + BEAT, 80.0, 760.0, "Every person confirmed as a subscriber", GREEN, 640.0),
            pop(&frame, BAR + BEAT, len, 80.0, 760.0, "Respects Telegram's rate limits", BLUE, 560.0),
            pop(&frame, BAR + 2.0 * BEAT, len, 80.0, 860.0, "Refusals explained, nothing skipped", "#ff9f0a", 610.0),
        ];
        fframes::svgr!(
            <g opacity={out}>
                <g opacity={o} transform={cam}>
                    {window(ctx, "app-running.png", "w-run", 240.0, 150.0, 1440.0)}
                </g>
                <g opacity={ticker_o}>
                    <rect x="1310" y="70" width="520" height="150" rx="30" fill="#0c0b16" fill-opacity="0.88" stroke="#ffffff" stroke-opacity="0.14" />
                    <text x="1350" y="125" font-family={font(600)} font-weight="600" font-size="30" fill={SOFT}>"Approved & confirmed"</text>
                    <g transform={format!("translate(1350 196) scale({tick_s}) translate(-1350 -196)")}>
                        <text x="1350" y="196" font-family={font(900)} font-weight="900" font-size="64" fill={GREEN}>{Box::leak(thousands(approved).into_boxed_str()) as &str}</text>
                    </g>
                    <text x="1790" y="196" text-anchor="end" font-family={font(700)} font-weight="700" font-size="40" fill={WHITE}>"✓"</text>
                </g>
                {callouts}
            </g>
        )
    }
}

#[derive(Debug)]
struct Proof;

impl Scene for Proof {
    fn duration(&self) -> Duration<'_> {
        frames(S_PROOF, S_OUTRO)
    }

    fn render_frame<'a>(&'a self, frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let len = S_OUTRO - S_PROOF;
        let out = exit(&frame, len, 0.22);
        let o = appear(&frame, 0.0, 0.3);
        // Land on the reconciliation table: the proof.
        let z = 1.0 + 0.55 * tween(&frame, 0.15, 1.3, 0.0, 1.0, &IN_OUT);
        // Puts the "Matches exactly" row (window y 884) at y 700 on screen.
        let cam = camera(z, 1150.0, 600.0, -35.0, -340.0 * tween(&frame, 0.15, 1.3, 0.0, 1.0, &IN_OUT));
        let hl = appear(&frame, 2.0 * BEAT, 0.35);
        let caption_o = appear(&frame, BEAT * 1.0, 0.4);
        fframes::svgr!(
            <g opacity={out}>
                <g opacity={o} transform={cam}>
                    {window(ctx, "app-report.png", "w-proof", 240.0, 150.0, 1440.0)}
                    // "Matches exactly ✓", outlined on the beat
                    <rect x="625" y="862" width="850" height="44" rx="10" fill={GREEN} fill-opacity={0.14 * hl} stroke={GREEN} stroke-width="3" stroke-opacity={hl} />
                </g>
                <g opacity={caption_o} transform={format!("translate(0 {})", spring_in(&frame, BEAT, 24.0))}>
                    <rect x="560" y="900" width="800" height="104" rx="52" fill="#04130c" fill-opacity="0.9" stroke={GREEN} stroke-opacity="0.5" />
                    {headline(960.0, 968.0, 46.0, 800, WHITE, "middle", "The math adds up. Every time.")}
                </g>
            </g>
        )
    }
}

#[derive(Debug)]
struct Outro;

impl Scene for Outro {
    fn duration(&self) -> Duration<'_> {
        frames(S_OUTRO, END)
    }

    fn render_frame<'a>(&'a self, frame: Frame, ctx: &FFramesContext<'a, '_>) -> Svgr<'a> {
        let t = frame.seconds();
        let icon_o = appear(&frame, 0.0, 0.3);
        let icon_s = 1.0 + tween(&frame, 0.0, 3.0, 0.3, 0.0, &SNAPPY) + 0.03 * kick_pulse(t + S_OUTRO);
        let icon = ctx
            .get_image("app-icon.png")
            .map(|img| {
                let size = 230.0 * icon_s;
                fframes::svgr!(<image href={img.href()} x={960.0 - size / 2.0} y={300.0 - size / 2.0} width={size} height={size} opacity={icon_o} />)
            })
            .unwrap_or_default();
        let title_o = appear(&frame, 0.15, 0.3);
        let plat_o = appear(&frame, BEAT, 0.4);
        let url_o = appear(&frame, 2.0 * BEAT, 0.4);
        let glow = 0.5 + 0.5 * kick_pulse(t + S_OUTRO);
        fframes::svgr!(
            <g>
                {icon}
                <g opacity={title_o} transform={format!("translate(0 {})", spring_in(&frame, 0.15, 50.0))}>
                    {headline(960.0, 570.0, 150.0, 900, WHITE, "middle", "Join Approver")}
                </g>
                <g opacity={plat_o} transform={format!("translate(0 {})", spring_in(&frame, BEAT, 30.0))}>
                    {headline(960.0, 660.0, 50.0, 600, SOFT, "middle", "Free for macOS & Windows · updates itself")}
                </g>
                <g opacity={url_o} transform={format!("translate(0 {})", spring_in(&frame, 2.0 * BEAT, 30.0))}>
                    <rect x="555" y="735" width="810" height="100" rx="50" fill="#ffffff" fill-opacity={0.1 + 0.04 * glow} stroke="#ffffff" stroke-opacity="0.35" />
                    {headline(960.0, 802.0, 44.0, 700, WHITE, "middle", "github.com/Lulzx/join-approver")}
                </g>
            </g>
        )
    }
}
