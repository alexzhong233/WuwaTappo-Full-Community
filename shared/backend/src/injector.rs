//! Unified frontend injector for both macOS and Windows.
//!
//! Replaces the historical Python pipeline (`inject_core.py`, `fullatlas_core.py`,
//! `assistant_core.py`, `fullatlas-integration.py`, `build_integration.py`, and
//! `integrate_assistant.py`) while rendering the desktop overlay modules directly from
//! `shared/frontend/` + `shared/contracts/assistant-settings.json`.

use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

const SETTINGS_CONTRACT: &str = include_str!("../../contracts/assistant-settings.json");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Platform {
    Mac,
    Windows,
}

impl Platform {
    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "mac" | "macos" => Ok(Self::Mac),
            "win" | "windows" => Ok(Self::Windows),
            other => Err(format!("未知注入目标平台：{other}（期望 mac 或 win）")),
        }
    }

    pub fn brand(self) -> &'static str {
        match self {
            Self::Mac => "WUWA TAPPO FULLATLAS / MAC",
            Self::Windows => "WUWA TAPPO / 后台助手",
        }
    }

    pub fn platform_label(self) -> &'static str {
        match self {
            Self::Mac => "macOS",
            Self::Windows => "Windows",
        }
    }

    pub fn startup_error_prefix(self) -> &'static str {
        match self {
            Self::Mac => "Mac 全图鉴启动失败：",
            Self::Windows => {
                "本地素材服务没有启动，请用桌面快捷方式重新打开；仍不行请查看安装目录下的 host.stderr.log。"
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PatchLogEntry {
    pub anchor: String,
    pub matches: usize,
}

#[derive(Default)]
pub struct PatchRecorder {
    pub entries: Vec<PatchLogEntry>,
}

impl PatchRecorder {
    pub fn replace(&mut self, text: &str, old: &str, new: &str) -> Result<String, String> {
        self.replace_n(text, old, new, 1)
    }

    pub fn replace_n(
        &mut self,
        text: &str,
        old: &str,
        new: &str,
        expected: usize,
    ) -> Result<String, String> {
        if old.is_empty() {
            return Err("替换锚点不能为空".into());
        }
        let actual = text.match_indices(old).count();
        if actual != expected {
            let preview: String = old.chars().take(100).collect();
            return Err(format!(
                "注入锚点匹配数量异常：'{preview}'（实际 {actual}，期望 {expected}）"
            ));
        }
        let preview: String = old.chars().take(100).collect();
        self.entries.push(PatchLogEntry {
            anchor: preview,
            matches: actual,
        });
        Ok(text.replace(old, new))
    }
}

fn patch_accessory_transform(
    rec: &mut PatchRecorder,
    text: &str,
    alias: &str,
) -> Result<String, String> {
    let pattern = format!(
        r"(?s)([A-Za-z0-9_$]+)\s*=\s*([A-Za-z0-9_$]+)\(\(\)\s*=>\s*\{{\s*const\s*\{{\s*x:\s*([A-Za-z0-9_$]+),\s*y:\s*([A-Za-z0-9_$]+)\s*\}}\s*=\s*{}\(\s*(.*?)\s*\)\s*;\s*return\s*\{{\s*transform:\s*`translate\(\$\{{[^}}]+\}}px,\s*\$\{{[^}}]+\}}px\)`\s*\}}\s*\}}\s*\)",
        regex::escape(alias)
    );
    let re = Regex::new(&pattern).map_err(|error| error.to_string())?;
    let Some(caps) = re.captures(text) else {
        return Err(format!("accessory transform binding missing: {alias}"));
    };
    let whole = caps.get(0).ok_or("accessory transform match missing")?.as_str();
    let name = &caps[1];
    let computed = &caps[2];
    let x = &caps[3];
    let y = &caps[4];
    let user = &caps[5];
    let declaration = if whole.trim_start().starts_with("const ") { "const " } else { "" };
    let replacement = "__DECL____NAME__=__COMP(()=>{const{x:__X,y:__Y}=__ALIAS(__USER);const __t=window.__WUWA_LOCAL_ATLAS__?.accessoryTransform?.(__USER);return __t?{transform:`translate(${__X+__t.dx}px, ${__Y+__t.dy}px) rotate(${__t.rotate}deg) scale(${__t.scale})`,transformOrigin:'50% 50%'}:{transform:`translate(${__X}px, ${__Y}px)`}})"
        .replace("__DECL__", declaration)
        .replace("__NAME__", name)
        .replace("__COMP", computed)
        .replace("__ALIAS", alias)
        .replace("__USER", user)
        .replace("__X", x)
        .replace("__Y", y);
    rec.replace(text, whole, &replacement)
}

fn capture<'a>(caps: &Captures<'a>, name: &str) -> &'a str {
    caps.name(name).map(|m| m.as_str()).unwrap_or("")
}

/// EmotionMenu 的动画循环在官方前端里一直自转，即使菜单收起也持续 requestAnimationFrame。
/// 这里给官方循环补上「空闲停帧」和 FPS 上限，并通过 watch 唤醒；不依赖 M.size（官方收起菜单
/// 并不会清空 M），只看菜单开关与用户设置。
fn patch_emotion_menu_raf(
    rec: &mut PatchRecorder,
    main: &str,
    watch_sym: &str,
) -> Result<String, String> {
    let pattern = r#"(?s)function\s+(?P<fn_name>[A-Za-z0-9_$]+)\(\s*(?P<ts_param>[A-Za-z0-9_$]+)\s*\)\s*\{\s*const\s+(?P<u_var>[A-Za-z0-9_$]+)\s*=\s*Math\.min\(\s*\(\s*(?P<ts_in_min>[A-Za-z0-9_$]+)\s*-\s*(?P<clock_var>[A-Za-z0-9_$]+)\s*\)\s*\/\s*1e3\s*,\s*\.028\s*\)\s*;\s*(?P<clock_assign>[A-Za-z0-9_$]+)\s*=\s*(?P<ts_assign>[A-Za-z0-9_$]+)\s*;\s*(?P<raf_inner>[\s\S]*?;)\s*(?P<raf_var>[A-Za-z0-9_$]+)\s*=\s*requestAnimationFrame\(\s*(?P<fn_in_raf>[A-Za-z0-9_$]+)\s*\)\s*\}\s*;?\s*return\s+(?P<on_mounted>[A-Za-z0-9_$]+)\(\s*\(\s*\)\s*=>\s*\{\s*(?P<clock_reset>[A-Za-z0-9_$]+)\s*=\s*performance\.now\(\s*\)\s*,\s*(?P<raf_start>[A-Za-z0-9_$]+)\s*=\s*requestAnimationFrame\(\s*(?P<fn_start>[A-Za-z0-9_$]+)\s*\)\s*,\s*window\.addEventListener\(\s*["']resize["']\s*,\s*(?P<resize_fn>[A-Za-z0-9_$]+)\s*\)\s*\}\s*\)\s*,\s*(?P<on_unmounted>[A-Za-z0-9_$]+)\(\s*\(\s*\)\s*=>\s*\{\s*cancelAnimationFrame\(\s*(?P<raf_stop>[A-Za-z0-9_$]+)\s*\)\s*,\s*window\.removeEventListener\(\s*["']resize["']\s*,\s*(?P<resize_off>[A-Za-z0-9_$]+)\s*\)\s*,\s*(?P<unmount_tail>[\s\S]*?)\s*\}\s*\)\s*,\s*(?P<render_head>\(\s*[A-Za-z0-9_$]+(?:\s*,\s*[A-Za-z0-9_$]+){0,2}\s*\)\s*=>\s*(?P<open_ref>[A-Za-z0-9_$.]+)\.value\s*\?)"#;
    let re = Regex::new(pattern).map_err(|error| error.to_string())?;
    let matches: Vec<Captures<'_>> = re.captures_iter(main).collect();
    if matches.len() != 1 {
        return Err(format!(
            "EmotionMenu 动画循环锚点数量异常：{}（期望 1）",
            matches.len()
        ));
    }
    let caps = &matches[0];
    let same = capture(caps, "fn_name") == capture(caps, "fn_in_raf")
        && capture(caps, "fn_name") == capture(caps, "fn_start")
        && capture(caps, "ts_param") == capture(caps, "ts_in_min")
        && capture(caps, "ts_param") == capture(caps, "ts_assign")
        && capture(caps, "clock_var") == capture(caps, "clock_assign")
        && capture(caps, "clock_var") == capture(caps, "clock_reset")
        && capture(caps, "raf_var") == capture(caps, "raf_start")
        && capture(caps, "raf_var") == capture(caps, "raf_stop")
        && capture(caps, "resize_fn") == capture(caps, "resize_off");
    if !same {
        return Err("EmotionMenu 动画循环变量绑定不一致".into());
    }
    let whole = caps.get(0).map(|m| m.as_str()).unwrap_or_default();
    let replacement = "let __wuwaRafLast = 0;\nfunction __FN__(__TS__) {\n const __p = __lg.perf?.value || {};\n if (__p.idlePause !== false && !__OPEN__.value) { __RAF__ = 0; return; }\n const __cap = Number(__p.fpsCap) || 0;\n if (!(__cap > 0 && __wuwaRafLast > 0 && __TS__ - __wuwaRafLast < 1000 / __cap - 1)) { __wuwaRafLast = __TS__; const __U__ = Math.min((__TS__ - __CLOCK__) / 1e3, .028); __CLOCK__ = __TS__; __INNER__ }\n __RAF__ = requestAnimationFrame(__FN__)\n}\nfunction __wuwaWakeRaf() { if (!__RAF__) { __CLOCK__ = performance.now(); __wuwaRafLast = 0; __RAF__ = requestAnimationFrame(__FN__); } }\n__WATCH__([__OPEN__, () => __lg.perf?.value?.idlePause], ([__open, __idle]) => { if (__open || __idle === false) __wuwaWakeRaf(); }, { flush: \"sync\" });\nreturn __MOUNTED__(() => { window.addEventListener(\"resize\", __RESIZE__); __wuwaWakeRaf(); }), __UNMOUNTED__(() => { cancelAnimationFrame(__RAF__); __RAF__ = 0; window.removeEventListener(\"resize\", __RESIZE__); __TAIL__ }), __HEAD__"
        .replace("__FN__", capture(caps, "fn_name"))
        .replace("__TS__", capture(caps, "ts_param"))
        .replace("__OPEN__", capture(caps, "open_ref"))
        .replace("__RAF__", capture(caps, "raf_var"))
        .replace("__CLOCK__", capture(caps, "clock_var"))
        .replace("__U__", capture(caps, "u_var"))
        .replace("__INNER__", capture(caps, "raf_inner"))
        .replace("__WATCH__", watch_sym)
        .replace("__MOUNTED__", capture(caps, "on_mounted"))
        .replace("__UNMOUNTED__", capture(caps, "on_unmounted"))
        .replace("__RESIZE__", capture(caps, "resize_fn"))
        .replace("__TAIL__", capture(caps, "unmount_tail"))
        .replace("__HEAD__", capture(caps, "render_head"));
    rec.replace(main, whole, &replacement)
}

/// 联机大厅的成员显示/位置布局全部落本机：可见性、恢复落点、跨窗口 arrange/reset 与拖拽保存。
fn patch_room_layout_hooks(
    rec: &mut PatchRecorder,
    main: &str,
    on_unmounted_sym: &str,
) -> Result<String, String> {
    // 钩子 A：成员可见性判断追加本机隐藏名单。
    let vis_re = Regex::new(
        r#"(?s)function\s+(?P<vo_fn>[A-Za-z0-9_$]+)\(\s*(?P<vo_arg>[A-Za-z0-9_$]+)\s*\)\s*\{\s*return\s+(?P<ho>[A-Za-z0-9_$]+)\.value\s*&&\s*(?P<vo_arg2>[A-Za-z0-9_$]+)\s*!==\s*(?P<ao>[A-Za-z0-9_$]+)\.value\s*\|\|\s*(?P<jo>[A-Za-z0-9_$]+)\.value\.has\(\s*(?P<vo_arg3>[A-Za-z0-9_$]+)\s*\)\s*\}"#,
    )
    .map_err(|error| error.to_string())?;
    let vis_matches: Vec<Captures<'_>> = vis_re.captures_iter(main).collect();
    if vis_matches.len() != 1 {
        return Err(format!(
            "联机成员可见性锚点数量异常：{}（期望 1）",
            vis_matches.len()
        ));
    }
    let vis = &vis_matches[0];
    let vo_arg = capture(vis, "vo_arg");
    if vo_arg != capture(vis, "vo_arg2") || vo_arg != capture(vis, "vo_arg3") {
        return Err("联机成员可见性变量绑定不一致".into());
    }
    let vis_whole = vis.get(0).map(|m| m.as_str()).unwrap_or_default();
    let vis_out = format!(
        "function {}({vo_arg}) {{ return {}.value && {vo_arg} !== {}.value || {}.value.has({vo_arg}) || Boolean(__lg.isRoomMemberHidden?.({vo_arg})) }}",
        capture(vis, "vo_fn"),
        capture(vis, "ho"),
        capture(vis, "ao"),
        capture(vis, "jo"),
    );
    let main = rec.replace(main, vis_whole, &vis_out)?;

    // 钩子 B：位置恢复 + 实时排列/重置监听。
    let layout_re = Regex::new(
        r#"(?s)function\s+(?P<ep_fn>[A-Za-z0-9_$]+)\(\s*(?P<ep_arg>[A-Za-z0-9_$]+)\s*\)\s*\{\s*if\s*\(\s*(?P<pt_set>[A-Za-z0-9_$]+)\.has\(\s*(?P<ep_arg2>[A-Za-z0-9_$]+)\.id\s*\)\s*\|\|\s*!\s*(?P<ready>[A-Za-z0-9_$]+)\.value\s*\)\s*return\s*;\s*const\s+(?P<t_var>[A-Za-z0-9_$]+)\s*=\s*(?P<ue_ref>[A-Za-z0-9_$]+)\.value\?\.querySelector\(\s*["']\.cat-wrap["']\s*\)\s*;\s*if\s*\(\s*!\s*(?P<t_var2>[A-Za-z0-9_$]+)\s*\)\s*return\s*;\s*const\s+(?P<o_var>[A-Za-z0-9_$]+)\s*=\s*(?P<t_var3>[A-Za-z0-9_$]+)\.getBoundingClientRect\(\s*\)\s*,\s*(?P<l_var>[A-Za-z0-9_$]+)\s*=\s*(?P<o_var2>[A-Za-z0-9_$]+)\.width\s*\/\s*(?P<base_w>[A-Za-z0-9_$]+)\s*;\s*if\s*\(\s*!\s*\(\s*(?P<l_var2>[A-Za-z0-9_$]+)\s*>\s*0\s*\)\s*\|\|\s*!\s*Number\.isFinite\(\s*(?P<o_var3>[A-Za-z0-9_$]+)\.left\s*\)\s*\|\|\s*!\s*Number\.isFinite\(\s*(?P<o_var4>[A-Za-z0-9_$]+)\.top\s*\)\s*\)\s*return\s*;\s*const\s+(?P<d_var>[A-Za-z0-9_$]+)\s*=\s*(?P<clamp_fn>[A-Za-z0-9_$]+)\(\s*(?P<o_var5>[A-Za-z0-9_$]+)\.left\s*\+\s*(?P<ep_arg3>[A-Za-z0-9_$]+)\.left\s*\*\s*(?P<l_var3>[A-Za-z0-9_$]+)\s*,\s*(?P<o_var6>[A-Za-z0-9_$]+)\.top\s*\+\s*(?P<ep_arg4>[A-Za-z0-9_$]+)\.top\s*\*\s*(?P<l_var4>[A-Za-z0-9_$]+)\s*\)\s*;\s*(?P<ep_arg5>[A-Za-z0-9_$]+)\.left\s*=\s*(?P<d_var2>[A-Za-z0-9_$]+)\.left\s*,\s*(?P<ep_arg6>[A-Za-z0-9_$]+)\.top\s*=\s*(?P<d_var3>[A-Za-z0-9_$]+)\.top\s*,\s*(?P<pt_set2>[A-Za-z0-9_$]+)\.add\(\s*(?P<ep_arg7>[A-Za-z0-9_$]+)\.id\s*\)\s*,\s*(?P<zi_fn>[A-Za-z0-9_$]+)\(\s*(?P<ep_arg8>[A-Za-z0-9_$]+)\s*\)\s*\}\s*async\s+function\s+(?P<ml_fn>[A-Za-z0-9_$]+)\(\s*\)\s*\{\s*if\s*\(\s*await\s+(?P<tick_fn>[A-Za-z0-9_$]+)\(\s*\)\s*,\s*!!\s*(?P<ready2>[A-Za-z0-9_$]+)\.value\s*\)\s*\{\s*for\s*\(\s*const\s*\[\s*(?P<k_var>[A-Za-z0-9_$]+)\s*,\s*(?P<v_var>[A-Za-z0-9_$]+)\s*\]\s*of\s*(?P<members_comp>[A-Za-z0-9_$]+)\.value\.entries\(\s*\)\s*\)\s*(?P<il_fn>[A-Za-z0-9_$]+)\(\s*(?P<v_var2>[A-Za-z0-9_$]+)\s*,\s*(?P<k_var2>[A-Za-z0-9_$]+)\s*\)\s*,\s*(?P<ep_fn2>[A-Za-z0-9_$]+)\(\s*(?P<v_var3>[A-Za-z0-9_$]+)\s*\)\s*;\s*(?P<ta_fn>[A-Za-z0-9_$]+)\(\s*\)\s*,\s*await\s+(?P<tick_fn2>[A-Za-z0-9_$]+)\(\s*\)\s*,\s*(?P<ea_fn>[A-Za-z0-9_$]+)\(\s*\)\s*\}\s*\}\s*function\s+(?P<jn_fn>[A-Za-z0-9_$]+)\(\s*\)\s*\{\s*(?P<ml_fn2>[A-Za-z0-9_$]+)\(\s*\)\s*\}"#,
    )
    .map_err(|error| error.to_string())?;
    let layout_matches: Vec<Captures<'_>> = layout_re.captures_iter(&main).collect();
    if layout_matches.len() != 1 {
        return Err(format!(
            "联机成员布局锚点数量异常：{}（期望 1）",
            layout_matches.len()
        ));
    }
    let lay = &layout_matches[0];
    let ep_arg = capture(lay, "ep_arg");
    let t_var = capture(lay, "t_var");
    let o_var = capture(lay, "o_var");
    let l_var = capture(lay, "l_var");
    let d_var = capture(lay, "d_var");
    let v_var = capture(lay, "v_var");
    let same = ep_arg == capture(lay, "ep_arg2")
        && ep_arg == capture(lay, "ep_arg3")
        && ep_arg == capture(lay, "ep_arg4")
        && ep_arg == capture(lay, "ep_arg5")
        && ep_arg == capture(lay, "ep_arg6")
        && ep_arg == capture(lay, "ep_arg7")
        && ep_arg == capture(lay, "ep_arg8")
        && t_var == capture(lay, "t_var2")
        && t_var == capture(lay, "t_var3")
        && o_var == capture(lay, "o_var2")
        && o_var == capture(lay, "o_var3")
        && o_var == capture(lay, "o_var4")
        && o_var == capture(lay, "o_var5")
        && o_var == capture(lay, "o_var6")
        && l_var == capture(lay, "l_var2")
        && l_var == capture(lay, "l_var3")
        && l_var == capture(lay, "l_var4")
        && d_var == capture(lay, "d_var2")
        && d_var == capture(lay, "d_var3")
        && capture(lay, "pt_set") == capture(lay, "pt_set2")
        && capture(lay, "ready") == capture(lay, "ready2")
        && capture(lay, "ep_fn") == capture(lay, "ep_fn2")
        && capture(lay, "ml_fn") == capture(lay, "ml_fn2")
        && capture(lay, "tick_fn") == capture(lay, "tick_fn2")
        && capture(lay, "k_var") == capture(lay, "k_var2")
        && v_var == capture(lay, "v_var2")
        && v_var == capture(lay, "v_var3");
    if !same {
        return Err("联机成员布局变量绑定不一致".into());
    }
    let layout_whole = lay.get(0).map(|m| m.as_str()).unwrap_or_default();
    let template = "function __EP__(__ARG__) { if (__SET__.has(__ARG__.id) || !__READY__.value) return; const __T__ = __UE__.value?.querySelector(\".cat-wrap\"); if (!__T__) return; const __O__ = __T__.getBoundingClientRect(), __L__ = __O__.width / __BASE__; if (!(__L__ > 0) || !Number.isFinite(__O__.left) || !Number.isFinite(__O__.top)) return; const __others = __MEMBERS__.value.filter(__m => __m.id !== __ARG__.id && __SET__.has(__m.id)).map(__m => ({ left: __m.left, top: __m.top })); const __saved = __lg.savedRoomMemberPosition?.(__ARG__.id, __others, Math.max(80, Math.round(132 * __L__)), Math.max(80, Math.round(152 * __L__))); const __D__ = __saved ? __CLAMP__(__saved.left, __saved.top) : __CLAMP__(__O__.left + __ARG__.left * __L__, __O__.top + __ARG__.top * __L__); __ARG__.left = __D__.left, __ARG__.top = __D__.top, __SET__.add(__ARG__.id), __ZI__(__ARG__) }\nasync function __ML__() { if (await __TICK__(), !!__READY__.value) { for (const [__K__, __V__] of __MEMBERS__.value.entries()) __IL__(__V__, __K__), __EP__(__V__); __TA__(), await __TICK__(), __EA__() } }\nfunction __JN__() { __ML__() }\nfunction __wuwaRoomStorage(event) {\n if (!event || event.key !== \"wuwatappo-room-action-v1\" || !event.newValue) return;\n let payload = null;\n try { payload = JSON.parse(event.newValue); } catch (error) { return; }\n if (!payload || typeof payload !== \"object\") return;\n if (payload.action === \"arrange\") {\n  const __visible = __MEMBERS__.value.filter(__m => !__lg.isRoomMemberHidden?.(__m.id));\n  const __positions = __lg.arrangeRoomMembers?.(__visible, payload.mode || \"horizontal\", { width: window.innerWidth, height: window.innerHeight }) || [];\n  for (const __pos of __positions) {\n   const __m = __MEMBERS__.value.find(__item => __item.id === __pos.id);\n   if (!__m) continue;\n   const __c = __CLAMP__(__pos.left, __pos.top);\n   __m.left = __c.left; __m.top = __c.top; __SET__.add(__m.id); __ZI__(__m); __lg.saveRoomMemberPosition?.(__m.id, __c.left, __c.top);\n  }\n  __JN__();\n } else if (payload.action === \"reset-positions\") {\n  for (const __m of __MEMBERS__.value) { __SET__.delete(__m.id); __m.left = 0; __m.top = 0; }\n  __JN__();\n }\n}\nwindow.addEventListener(\"storage\", __wuwaRoomStorage);\n__UNMOUNTED__(() => window.removeEventListener(\"storage\", __wuwaRoomStorage));";
    let layout_out = template
        .replace("__EP__", capture(lay, "ep_fn"))
        .replace("__ARG__", ep_arg)
        .replace("__SET__", capture(lay, "pt_set"))
        .replace("__READY__", capture(lay, "ready"))
        .replace("__T__", t_var)
        .replace("__UE__", capture(lay, "ue_ref"))
        .replace("__O__", o_var)
        .replace("__L__", l_var)
        .replace("__BASE__", capture(lay, "base_w"))
        .replace("__CLAMP__", capture(lay, "clamp_fn"))
        .replace("__D__", d_var)
        .replace("__ZI__", capture(lay, "zi_fn"))
        .replace("__MEMBERS__", capture(lay, "members_comp"))
        .replace("__ML__", capture(lay, "ml_fn"))
        .replace("__TICK__", capture(lay, "tick_fn"))
        .replace("__K__", capture(lay, "k_var"))
        .replace("__V__", v_var)
        .replace("__IL__", capture(lay, "il_fn"))
        .replace("__TA__", capture(lay, "ta_fn"))
        .replace("__EA__", capture(lay, "ea_fn"))
        .replace("__JN__", capture(lay, "jn_fn"))
        .replace("__UNMOUNTED__", on_unmounted_sym);
    let main = rec.replace(&main, layout_whole, &layout_out)?;

    // 钩子 C：拖拽结束后落点入库。
    let drag_re = Regex::new(
        r#"(?s)(?P<drag_obj>[A-Za-z0-9_$]+)\s*&&\s*(?P<drag_test>[A-Za-z0-9_$]+)\(\s*(?P<drag_obj2>[A-Za-z0-9_$]+)\s*\)\s*&&\s*\(\s*(?P<bounce>[A-Za-z0-9_$]+)\s*\?\s*(?P<settle>[A-Za-z0-9_$]+)\(\s*(?P<drag_obj3>[A-Za-z0-9_$]+)\s*,\s*\{\s*left:\s*(?P<left_val>[A-Za-z0-9_$]+)\s*,\s*top:\s*(?P<top_val>[A-Za-z0-9_$]+)\s*\}\s*\)\s*:\s*\(\s*(?P<drag_obj4>[A-Za-z0-9_$]+)\.left\s*=\s*(?P<pos_var>[A-Za-z0-9_$]+)\.left\s*,\s*(?P<drag_obj5>[A-Za-z0-9_$]+)\.top\s*=\s*(?P<pos_var2>[A-Za-z0-9_$]+)\.top\s*,\s*(?P<settle2>[A-Za-z0-9_$]+)\(\s*(?P<drag_obj6>[A-Za-z0-9_$]+)\s*,\s*(?P<pos_var3>[A-Za-z0-9_$]+)\s*\)\s*\)\s*\)\s*;"#,
    )
    .map_err(|error| error.to_string())?;
    let drag_matches: Vec<Captures<'_>> = drag_re.captures_iter(&main).collect();
    if drag_matches.len() != 1 {
        return Err(format!(
            "联机成员拖拽锚点数量异常：{}（期望 1）",
            drag_matches.len()
        ));
    }
    let drag = &drag_matches[0];
    let drag_obj = capture(drag, "drag_obj");
    let pos_var = capture(drag, "pos_var");
    let same = drag_obj == capture(drag, "drag_obj2")
        && drag_obj == capture(drag, "drag_obj3")
        && drag_obj == capture(drag, "drag_obj4")
        && drag_obj == capture(drag, "drag_obj5")
        && drag_obj == capture(drag, "drag_obj6")
        && pos_var == capture(drag, "pos_var2")
        && pos_var == capture(drag, "pos_var3");
    if !same {
        return Err("联机成员拖拽变量绑定不一致".into());
    }
    let drag_whole = drag.get(0).map(|m| m.as_str()).unwrap_or_default();
    let bounce = capture(drag, "bounce");
    let left_val = capture(drag, "left_val");
    let top_val = capture(drag, "top_val");
    let drag_out = format!(
        "{drag_whole} if ({drag_obj} && {}({drag_obj})) __lg.saveRoomMemberPosition?.({drag_obj}.id, {bounce} ? {left_val} : {pos_var}.left, {bounce} ? {top_val} : {pos_var}.top);",
        capture(drag, "drag_test")
    );
    rec.replace(&main, drag_whole, &drag_out)
}

/// 联机房间的 `syncEmotion`（`wt("re", { e: emoteId })`）只发给官方服务端，
/// 本机 Mod 表情 ID 不在服务端字典里，发送会返回 `code.1`（「网络繁忙」）。
fn patch_sync_emotion(rec: &mut PatchRecorder, main: &str) -> Result<String, String> {
    let re = Regex::new(
        r#"(?s)function\s+([A-Za-z0-9_$]+)\(\s*([A-Za-z0-9_$]+)\s*\)\s*\{\s*([A-Za-z0-9_$]+)\.value\s*&&\s*([A-Za-z0-9_$]+)\(\s*["']re["']\s*,\s*\{\s*e:\s*([A-Za-z0-9_$]+)\s*\}\s*\)\s*\}"#,
    )
    .unwrap();
    let matches: Vec<Captures<'_>> = re.captures_iter(main).collect();
    if matches.len() != 1 {
        return Err(format!(
            "房间同步表情锚点数量异常：{}（期望 1）",
            matches.len()
        ));
    }
    let caps = &matches[0];
    if &caps[2] != &caps[5] {
        return Err("房间同步表情参数绑定不一致".into());
    }
    let whole = caps.get(0).unwrap().as_str();
    let replacement = whole.replacen(
        '{',
        &format!("{{ if (__lg.isModAsset?.({arg})) return;", arg = &caps[2]),
        1,
    );
    rec.replace(main, whole, &replacement)
}

fn patch_monitor_switch_display(rec: &mut PatchRecorder, monitor: &str) -> Result<String, String> {
    let re = Regex::new(
        r#"async function\s+([A-Za-z0-9_$]+)\(\)\s*\{\s*if\s*\(\s*([A-Za-z0-9_$]+)\s*\)\s*return\s+([A-Za-z0-9_$]+)\s*;\s*([A-Za-z0-9_$]+)\s*=\s*([A-Za-z0-9_$]+)\(\)\s*;\s*try\s*\{\s*return await\s+([A-Za-z0-9_$]+)\s*;?\s*\}\s*finally\s*\{\s*([A-Za-z0-9_$]+)\s*=\s*null\s*;?\s*\}\s*\}"#,
    )
    .unwrap();
    let caps = re
        .captures_iter(monitor)
        .find(|c| {
            let cache = &c[2];
            let inner = &c[5];
            let needle = format!("async function {inner}(");
            cache == &c[3]
                && cache == &c[4]
                && cache == &c[6]
                && cache == &c[7]
                && monitor.find(&needle).is_some_and(|idx| {
                    let tail = &monitor[idx + needle.len()..];
                    let end = tail
                        .find("\nasync function ")
                        .map(|d| idx + needle.len() + d)
                        .unwrap_or(monitor.len());
                    monitor[idx..end].contains("\"set_main_window_monitor_bounds\"")
                })
        })
        .ok_or("缺少 switchDisplayMonitor 切屏入口锚点")?;
    let whole = caps[0].to_owned();
    let fn_name = &caps[1];
    rec.replace(
        monitor,
        &whole,
        &format!(
            "{whole}\nif (typeof window !== \"undefined\") window.__WUWA_SWITCH_DISPLAY__ = {fn_name};"
        ),
    )
}

/// Convert `inspect_js.cjs` Unicode codepoint (char) indices into UTF-8 byte offsets.
pub fn char_to_byte_map(text: &str) -> Vec<usize> {
    let mut map = Vec::with_capacity(text.chars().count() + 1);
    for (byte_idx, _) in text.char_indices() {
        map.push(byte_idx);
    }
    map.push(text.len());
    map
}

fn map_offset(map: &[usize], char_offset: usize, label: &str) -> Result<usize, String> {
    map.get(char_offset)
        .copied()
        .ok_or_else(|| format!("AST 偏移量越界 ({label}): {char_offset} >= {}", map.len()))
}

#[derive(Clone, Debug, Deserialize)]
pub struct FunctionMeta {
    pub name: String,
    pub start: usize,
    pub end: usize,
    pub body: usize,
    #[serde(rename = "bodyEnd")]
    pub body_end: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ComponentMeta {
    pub name: String,
    pub start: usize,
    pub end: usize,
    #[serde(default)]
    pub variable: Option<String>,
    pub body: usize,
    #[serde(rename = "bodyEnd")]
    pub body_end: usize,
    #[serde(rename = "returnStart", default)]
    pub return_start: Option<usize>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ContainerMeta {
    pub start: usize,
    #[allow(dead_code)]
    pub end: usize,
    pub array: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RoomConditionMeta {
    pub start: usize,
    #[allow(dead_code)]
    pub end: usize,
    pub test: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SnapshotMeta {
    pub start: usize,
    pub end: usize,
    pub fields: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct FileMeta {
    #[serde(default)]
    pub imports: BTreeMap<String, String>,
    #[serde(default)]
    pub functions: Vec<FunctionMeta>,
    #[serde(default)]
    pub components: Vec<ComponentMeta>,
    #[serde(default)]
    pub containers: Vec<ContainerMeta>,
    #[serde(rename = "roomConditions", default)]
    pub room_conditions: Vec<RoomConditionMeta>,
    #[serde(default)]
    pub cases: BTreeMap<String, String>,
    #[serde(default)]
    pub snapshots: Vec<SnapshotMeta>,
}

impl FileMeta {
    fn convert_offsets_to_bytes(&mut self, map: &[usize]) -> Result<(), String> {
        for f in &mut self.functions {
            f.start = map_offset(map, f.start, &f.name)?;
            f.end = map_offset(map, f.end, &f.name)?;
            f.body = map_offset(map, f.body, &f.name)?;
            f.body_end = map_offset(map, f.body_end, &f.name)?;
        }
        for c in &mut self.components {
            c.start = map_offset(map, c.start, &c.name)?;
            c.end = map_offset(map, c.end, &c.name)?;
            c.body = map_offset(map, c.body, &c.name)?;
            c.body_end = map_offset(map, c.body_end, &c.name)?;
            if let Some(rs) = c.return_start {
                c.return_start = Some(map_offset(map, rs, &c.name)?);
            }
        }
        for container in &mut self.containers {
            container.start = map_offset(map, container.start, "container")?;
            container.end = map_offset(map, container.end, "container")?;
            container.array = map_offset(map, container.array, "container.array")?;
        }
        for cond in &mut self.room_conditions {
            cond.start = map_offset(map, cond.start, "roomCondition")?;
            cond.end = map_offset(map, cond.end, "roomCondition")?;
        }
        for snap in &mut self.snapshots {
            snap.start = map_offset(map, snap.start, "snapshot")?;
            snap.end = map_offset(map, snap.end, "snapshot")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct JsStructure {
    pub shared: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub exports: BTreeMap<String, String>,
    pub files: BTreeMap<String, FileMeta>,
}

#[derive(Clone, Debug)]
pub struct Edit {
    pub start: usize,
    pub end: usize,
    pub value: String,
}

impl Edit {
    pub fn new(start: usize, end: usize, value: impl Into<String>) -> Self {
        Self {
            start,
            end,
            value: value.into(),
        }
    }
}

/// Apply span edits in reverse offset order so earlier indices stay valid.
pub fn apply_edits(text: &str, edits: Vec<Edit>) -> Result<String, String> {
    let mut indexed: Vec<(usize, Edit)> = edits.into_iter().enumerate().collect();
    // Preserve reverse tie-break behavior on identical `(start, end)` if ever present.
    indexed.sort_by(|(idx_a, a), (idx_b, b)| {
        b.start
            .cmp(&a.start)
            .then_with(|| b.end.cmp(&a.end))
            .then_with(|| idx_b.cmp(idx_a))
    });
    let mut out = text.to_owned();
    let mut last = out.len();
    for (_, edit) in indexed {
        if !(edit.start <= edit.end && edit.end <= last) {
            return Err(format!(
                "编辑区间无效或重叠：({}, {}), last={last}",
                edit.start, edit.end
            ));
        }
        if !out.is_char_boundary(edit.start) || !out.is_char_boundary(edit.end) {
            return Err(format!(
                "编辑区间不在 UTF-8 字符边界上：({}, {})",
                edit.start, edit.end
            ));
        }
        out.replace_range(edit.start..edit.end, &edit.value);
        last = edit.start;
    }
    Ok(out)
}

pub fn function_for<'a>(
    functions: &'a [FunctionMeta],
    text: &str,
    needle: &str,
) -> Result<&'a FunctionMeta, String> {
    functions
        .iter()
        .filter(|f| {
            text.get(f.start..f.end)
                .is_some_and(|slice| slice.contains(needle))
        })
        .min_by_key(|f| f.end.saturating_sub(f.start))
        .ok_or_else(|| format!("未找到包含特征 '{needle}' 的函数"))
}

pub fn named<'a>(functions: &'a [FunctionMeta], name: &str) -> Result<&'a FunctionMeta, String> {
    let mut matches = functions.iter().filter(|f| f.name == name);
    let first = matches
        .next()
        .ok_or_else(|| format!("未找到命名函数：{name}"))?;
    if matches.next().is_some() {
        return Err(format!("命名函数不唯一：{name}"));
    }
    Ok(first)
}

/// 在已解析的 AST 函数区间（优先组件内部函数，再回退顶层函数）里读取首个形参名，
/// 避免同名函数出现在文件更靠前位置时被全局正则误匹配。
pub fn fn_param_in_scope(
    text: &str,
    component_functions: &[FunctionMeta],
    all_functions: &[FunctionMeta],
    fn_name: &str,
) -> Result<Option<String>, String> {
    let meta = named(component_functions, fn_name).or_else(|_| named(all_functions, fn_name))?;
    let header = text
        .get(meta.start..meta.body)
        .ok_or_else(|| format!("函数 {fn_name} 区间越界"))?;
    let re = Regex::new(&format!(
        r"(?:async\s+)?function\s+{}\(\s*([A-Za-z0-9_$]+)",
        regex::escape(fn_name)
    ))
    .unwrap();
    Ok(re.captures(header).map(|c| c[1].to_owned()))
}

fn require_import<'a>(imports: &'a BTreeMap<String, String>, key: &str) -> Result<&'a str, String> {
    imports
        .get(key)
        .map(String::as_str)
        .ok_or_else(|| format!("js_structure 缺少导入绑定：{key}"))
}

fn with_alias(imports: &mut BTreeMap<String, String>, canonical: &str) {
    if imports.contains_key(canonical) {
        return;
    }
    let alt = format!("{canonical}$1");
    if let Some(value) = imports.get(&alt).cloned() {
        imports.insert(canonical.to_owned(), value);
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let target = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Render a module directly from `shared/frontend/<name>` for the target platform,
/// substituting `__WUWA_BRAND__`, `__WUWA_PLATFORM__`, and `__WUWA_ASSISTANT_DEFAULTS__`.
pub fn render_frontend_module(
    repo_root: &Path,
    name: &str,
    platform: Platform,
) -> Result<String, String> {
    let source_path = repo_root.join("shared/frontend").join(name);
    let raw = fs::read_to_string(&source_path)
        .map_err(|e| format!("读取共享前端模块 {} 失败：{e}", source_path.display()))?;
    render_frontend_source(name, &raw, platform)
}

pub fn render_frontend_source(
    name: &str,
    raw: &str,
    platform: Platform,
) -> Result<String, String> {
    let mut rendered = raw
        .replace("__WUWA_BRAND__", platform.brand())
        .replace("__WUWA_PLATFORM__", platform.platform_label());
    if name == "desktop_assistant.js" {
        let contract: Value = serde_json::from_str(SETTINGS_CONTRACT)
            .map_err(|e| format!("解析 assistant-settings.json 失败：{e}"))?;
        let defaults = contract
            .get("defaults")
            .ok_or("assistant-settings.json 缺少 defaults")?;
        let defaults_json = serde_json::to_string(defaults).map_err(|e| e.to_string())?;
        rendered = rendered.replace("__WUWA_ASSISTANT_DEFAULTS__", &defaults_json);
    }
    for placeholder in [
        "__WUWA_BRAND__",
        "__WUWA_PLATFORM__",
        "__WUWA_ASSISTANT_DEFAULTS__",
    ] {
        if rendered.contains(placeholder) {
            return Err(format!("{name} 渲染后仍保留占位符 {placeholder}"));
        }
    }
    Ok(rendered)
}

const MOD_NAME_FALLBACK_OLD: &str = r#"function resolveAssetDisplayName(e, t) {
  const n = t?.trim() ?? "";
  if (!n) return "";
  const s = String(e ?? "").trim().toLowerCase();
  return s === "character_skin" || s === "character_skin_atlas" ? resolveCharacterSkinDisplayName(n) : s === "accessory" || s === "accessory_atlas" ? resolveAccessoryDisplayName(n) : s === "emote" || s === "emote_atlas" ? resolveEmoteDisplayName(n) : ""
}"#;

const MOD_NAME_FALLBACK_NEW: &str = r#"function resolveAssetDisplayName(e, t) {
  const n = t?.trim() ?? "";
  if (!n) return "";
  const s = String(e ?? "").trim().toLowerCase();
  const resolved = s === "character_skin" || s === "character_skin_atlas" ? resolveCharacterSkinDisplayName(n) : s === "accessory" || s === "accessory_atlas" ? resolveAccessoryDisplayName(n) : s === "emote" || s === "emote_atlas" ? resolveEmoteDisplayName(n) : "";
  // 本机 Mod 的名字不在官方远端字典里：查不到就直接用条目名，否则图鉴/背包只剩空白或占位符。
  const mods = typeof window !== "undefined" ? window.__WUWA_MOD_NAMES__ : null;
  return resolved || (mods && mods.has(n) ? n : "")
}"#;

pub fn patch_mod_display_name(rec: &mut PatchRecorder, text: &str) -> Result<String, String> {
    rec.replace(text, MOD_NAME_FALLBACK_OLD, MOD_NAME_FALLBACK_NEW)
}

pub fn patch_mod_asset_url(rec: &mut PatchRecorder, text: &str) -> Result<String, String> {
    let re = Regex::new(r"function buildAssetUrl\(([^)]*)\)\s*\{").unwrap();
    let caps = re
        .captures(text)
        .ok_or("official buildAssetUrl contract changed")?;
    let whole = caps.get(0).unwrap().as_str().to_owned();
    let params: Vec<&str> = caps[1].split(',').map(str::trim).collect();
    if params.len() != 2 || params.iter().any(|p| p.is_empty()) {
        return Err(format!("buildAssetUrl 参数列表异常：{:?}", params));
    }
    let injection = format!(
        "{whole}\n  const modAssetPath = (typeof window !== \"undefined\" && window.__WUWA_MOD_ASSET_PATH__) ? window.__WUWA_MOD_ASSET_PATH__({}) : \"\";\n  if (modAssetPath) return modAssetPath;",
        params[1]
    );
    rec.replace(text, &whole, &injection)
}

pub fn patch_online_services(rec: &mut PatchRecorder, text: &str) -> Result<String, String> {
    let old = "loaderModule.init(kurodataConfig);\nwindow.kurodata = loaderModule;\ninitTrackCommonProps();\ninitFrontendAlertReporting();";
    let new = r#"let localOnlineServicesStarted = false;
function localStartOnlineServices() {
 if (localOnlineServicesStarted) return;
 localOnlineServicesStarted = true;
 loaderModule.init(kurodataConfig); window.kurodata = loaderModule;
 initTrackCommonProps(); initFrontendAlertReporting();
}"#;
    rec.replace(text, old, new)
}

pub fn patch_display_version(
    text: &str,
    community_version: &str,
    official_version: &str,
) -> Result<String, String> {
    let community_json = serde_json::to_string(community_version.trim()).unwrap();
    let official_json = serde_json::to_string(official_version.trim()).unwrap();

    let telemetry_re = Regex::new(r"cachedAppVersion\s*=\s*await getVersion\(\)").unwrap();
    let telemetry_count = telemetry_re.find_iter(text).count();
    if telemetry_count != 1 {
        return Err(format!(
            "cachedAppVersion telemetry anchor count={telemetry_count} (expected 1)"
        ));
    }
    let text = telemetry_re
        .replace(text, format!("cachedAppVersion = {official_json}").as_str())
        .into_owned();

    let version_re =
        Regex::new(r#"async function getVersion\(\)\s*\{\s*return invoke(?:\$1)?\("plugin:app\|version"\);?\s*\}"#)
            .unwrap();
    let version_count = version_re.find_iter(&text).count();
    if version_count != 1 {
        return Err(format!(
            "official getVersion anchor count={version_count} (expected 1)"
        ));
    }
    let replacement = format!(
        "async function getVersion() {{\n  let c = {community_json};\n  try {{\n    const r = await window.__WUWA_LOCAL_ATLAS__?.request?.(\"/community/version\", {{}});\n    if (r && typeof r.current === \"string\" && r.current.trim()) c = r.current.trim();\n  }} catch {{}}\n  const b = {official_json};\n  return b ? c + \" · 基底 \" + b : c;\n}}"
    );
    Ok(version_re.replace(&text, replacement.as_str()).into_owned())
}

pub fn patch_invoke_guard(
    rec: &mut PatchRecorder,
    text: &str,
    strict: bool,
) -> Result<String, String> {
    let mut found = Vec::new();
    for name in ["invoke$1", "invoke"] {
        let anchor = format!("async function {name}(e, t = {{}}, n) {{");
        if text.contains(&anchor) {
            found.push(anchor);
        }
    }
    if strict && found.is_empty() {
        return Err("tauri invoke wrapper contract missing".into());
    }
    let mut out = text.to_owned();
    for anchor in found {
        let replacement = format!(
            "{anchor}\n window.__WUWA_LOCAL_ATLAS__?.checkInvoke(e, t);"
        );
        out = rec.replace(&out, &anchor, &replacement)?;
    }
    Ok(out)
}

pub fn patch_taskbar_default(rec: &mut PatchRecorder, text: &str) -> Result<String, String> {
    let taskbar_default = "showTaskbarIcon: !0";
    let clamp_anchor = "function clampPetScale(e) {";
    if text.matches(taskbar_default).count() != 1 || text.matches(clamp_anchor).count() != 1 {
        eprintln!("warn: 官方前端里找不到任务栏图标默认值，跳过「默认隐藏」这一项");
        return Ok(text.to_owned());
    }
    let out = rec.replace(text, taskbar_default, "showTaskbarIcon: !1")?;
    let migration = r#"// 社区版：默认不占任务栏/程序坞图标；历史设置只迁移一次，避免每次启动覆盖用户选择。
(function () {
  try {
    const key = "wuwatappo-settings", marker = "wuwatappo-settings-community-taskbar-v1";
    if (window.localStorage.getItem(marker)) return;
    const saved = JSON.parse(window.localStorage.getItem(key) || "null");
    if (saved && typeof saved === "object" && "showTaskbarIcon" in saved) {
      saved.showTaskbarIcon = false;
      window.localStorage.setItem(key, JSON.stringify(saved));
    }
    window.localStorage.setItem(marker, "1");
  } catch (error) {
    console.warn("[community] taskbar default skipped", error);
  }
})();

function clampPetScale(e) {"#;
    rec.replace(&out, clamp_anchor, migration)
}

pub const ACTIVITY_LOADING_GUARD: &str = r#"
 // 活动配置是后台刷新（挂机开箱、切页、重连都会触发），不该用全屏遮罩。
 if (t && typeof t.url === "string" && t.url.indexOf("api/common/activity-config") !== -1) t.loading = !1;"#;

pub fn patch_atlas_guards(
    rec: &mut PatchRecorder,
    text: &str,
    extra_interceptor: &str,
) -> Result<String, String> {
    // Local resources augment the official catalog; they must never replace
    // the official catalog fetch or prevent the official network catalog from
    // applying. The panel snapshot merges only missing local entries later.
    let text = text.to_owned();
    let interceptor = format!(
        "this.instance.interceptors.request.use(t => {{\n if (window.__WUWA_LOCAL_ATLAS__?.enabled.value && window.__WUWA_LOCAL_ATLAS__.restricted(t.url,t.method)) throw window.__WUWA_LOCAL_ATLAS__.offlineError();{ACTIVITY_LOADING_GUARD}{extra_interceptor}"
    );
    rec.replace(
        &text,
        "this.instance.interceptors.request.use(t => {",
        &interceptor,
    )
}

fn prepare_mac_shared(rec: &mut PatchRecorder, shared: &str) -> Result<String, String> {
    let mut out = shared.to_owned();
    for name in [
        "resolveAppearanceAssetUrl",
        "resolveAppearanceAssetDisplayUrl",
        "resolveAppearanceAtlasUrl",
    ] {
        let sig_re = Regex::new(&format!(r"function {name}\([^\n]*\) \{{")).unwrap();
        let m = sig_re
            .find(&out)
            .ok_or_else(|| format!("Mac shared patch missing function {name}"))?;
        let start = m.start();
        let after_sig = &out[m.end()..];
        let mut end = (start + 3500).min(out.len());
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        let next_fn = ["\nfunction ", "\nasync function "]
            .into_iter()
            .filter_map(|needle| after_sig.find(needle).map(|off| m.end() + off))
            .min();
        if let Some(boundary) = next_fn {
            if boundary <= end {
                end = boundary;
            }
        }
        let original = out[start..end].to_owned();
        if original[1..].contains("\nfunction ") {
            return Err(format!("{name} regex matched past function boundary"));
        }
        let updated = original.replace("/^https?:\\/\\//i", "/^(?:https?|asset):\\/\\//i");
        if updated == original {
            return Err(format!("{name} did not contain https scheme regex"));
        }
        out = rec.replace(&out, &original, &updated)?;
    }
    Ok(out)
}

fn transport_options(platform: Platform, config: &Value) -> Result<String, String> {
    match platform {
        Platform::Mac => Ok(concat!(
            "origin: window.__WUWATAPPO_PROFILE__.assetHost, ",
            "request: (route,payload) => window.__TAURI_INTERNALS__.invoke('fullatlas_request',{action:route.replace(/^\\//,''),payload:payload||{}}).catch(e=>{throw e instanceof Error?e:new Error(typeof e==='string'?e:String(e?.message||e||'本地服务请求失败'));}), ",
            "token: null, customAssetUrl: () => \"\""
        )
        .to_owned()),
        Platform::Windows => {
            let port = config
                .get("port")
                .and_then(Value::as_u64)
                .ok_or("local_config.json 缺少有效的 port")?;
            let origin = serde_json::to_string(&format!("http://127.0.0.1:{port}")).unwrap();
            Ok(format!(
                "origin: {origin}, token: null, customAssetUrl: reference => {origin} + \"/\" + reference.split(\"/\").map(encodeURIComponent).join(\"/\")"
            ))
        }
    }
}

fn controller_init(platform: Platform, config: &Value, controller: &str) -> Result<String, String> {
    let transport = transport_options(platform, config)?;
    let prefix_json = serde_json::to_string(platform.startup_error_prefix()).unwrap();
    let api_base = serde_json::to_string(
        config.get("api_base").and_then(Value::as_str).unwrap_or(""),
    ).unwrap();
    Ok(format!(
        "{controller}\nconst localAtlasRoute = ['/', '/panel'].includes((window.location.hash || '#/').slice(1).split('?')[0]);\n\
let localAtlasController = null;\n\
const __wuwaModAssetIds = new Set(), __wuwaModAtlasIds = new Set();\n\
if (localAtlasRoute) try {{\n\
 localAtlasController = createLocalAtlasController({{\n\
ref, window, storage: localStorage, {transport}, normalizeAtlasList, resolvePetAssetIdFromAppearanceId, getAppearanceAssetById,\n\
apiBase: {api_base}, synthesisAtlas, synthesisAtlasSame, refreshBag: getBagList,\n\
fetchBag: async () => {{ const raw = unwrapApiData(await getBagList()); return normalizeBagList(Array.isArray(raw) ? raw : Array.isArray(raw?.list) ? raw.list : []); }},\n\
resolveAssetUrl: (item, host) => resolveBagAppearanceImageUrl(host || loadedAssetsHost || '', item),\n\
onSynthesisComplete: () => {{ invoke('plugin:event|emit',{{event:'cat-panel-action',payload:{{type:'synthesis-success'}}}}).catch(()=>{{}}); }},\n\
snapshotCatalog: () => ({{assets: appearanceAssetCatalog.value, atlases: appearanceAtlasCatalog.value, host: loadedAssetsHost, ts: loadedAssetsJsonTs}}),\n\
getAccountCumulative: () => globalStore.userInfo ? isFrozenClickCount(globalStore.serverClickTotal) ? globalStore.serverClickTotal : clampClickCountSum(globalStore.serverClickTotal+globalStore.localClickCount) : parseSafeClickCount(globalStore.localClickCount),\n\
getAccountPending: () => parseSafeClickCount(globalStore.localClickCount),\n\
catalogConsumer: localAtlasRoute,\n\
applyOfflineCatalog: (payload, host) => applyCatalog(buildCatalogMaps(payload), host, Date.now()),\n\
mergeModCatalog: (payload) => {{\n\
 const mod = payload ? buildCatalogMaps(payload) : {{ assets: new Map(), atlases: new Map() }};\n\
 const assets = new Map(appearanceAssetCatalog.value);\n\
 const atlases = new Map(appearanceAtlasCatalog.value);\n\
 let changed = false;\n\
 for (const id of __wuwaModAssetIds) {{ if (!mod.assets.has(id) && assets.delete(id)) changed = true; }}\n\
 for (const id of __wuwaModAtlasIds) {{ if (!mod.atlases.has(id) && atlases.delete(id)) changed = true; }}\n\
 __wuwaModAssetIds.clear(); __wuwaModAtlasIds.clear();\n\
 for (const [k, v] of mod.assets) {{\n\
  __wuwaModAssetIds.add(k);\n\
  const cur = assets.get(k);\n\
  if (!cur || cur.url !== v.url || cur.body1?.url !== v.body1?.url || cur.head1?.url !== v.head1?.url || cur.body2?.url !== v.body2?.url || cur.head2?.url !== v.head2?.url || cur.body3?.url !== v.body3?.url) {{\n\
   assets.set(k, v);\n\
   changed = true;\n\
  }}\n\
 }}\n\
 for (const [k, v] of mod.atlases) {{\n\
  __wuwaModAtlasIds.add(k);\n\
  const cur = atlases.get(k);\n\
  if (!cur || cur.url !== v.url || cur.height_light_id !== v.height_light_id) {{\n\
   atlases.set(k, v);\n\
   changed = true;\n\
  }}\n\
 }}\n\
 if (!changed) return;\n\
 applyCatalog({{ assets, atlases }}, loadedAssetsHost, loadedAssetsJsonTs);\n\
}},\n\
restoreCatalog: saved => applyCatalog({{assets: saved.assets, atlases: saved.atlases}}, saved.host, saved.ts),\n\
onError: error => console.error('[local-gallery]', error)\n\
}});\n\
}} catch (error) {{ console.error('[local-gallery] controller init failed; using official UI', error); }}\n\
window.__WUWA_LOCAL_ATLAS__ = localAtlasController;\n\
if (localAtlasController) {{\n\
localAtlasController.ready.then(async () => {{ localStartOnlineServices(); await bootstrap(); localAtlasController.startOverlay(); }}).catch(error => {{\n\
 console.error('[local-gallery] startup failed; falling back to official UI', {prefix_json}, error);\n\
 localAtlasController.enabled.value = false;\n\
 localStartOnlineServices();\n\
 return bootstrap();\n\
}});\n\
watch(localAtlasController.enabled, on => {{ if (!on) localStartOnlineServices(); }});\n\
watch(appearanceAssetCatalog, () => {{ localAtlasController?.syncModCatalog?.(); }});\n\
}} else {{ localStartOnlineServices(); bootstrap(); }}\n",
        api_base = api_base
    ))
}

fn find_component<'a>(meta: &'a FileMeta, name: &str) -> Result<&'a ComponentMeta, String> {
    meta.components
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| format!("缺少组件 {name}"))
}

fn find_char_boundary_backward(text: &str, pos: usize) -> usize {
    let mut i = pos.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

struct WindowsContextMenuPatch {
    context_actions: String,
}

fn patch_windows_context_menu(
    main: &str,
    source: &str,
    main_meta: &FileMeta,
    mi: &BTreeMap<String, String>,
    open_tab: &str,
) -> Result<(String, WindowsContextMenuPatch), String> {
    let popup_re = Regex::new(
        r#"await ([A-Za-z0-9_$]+)\("popup_main_dock_context_menu", \{\s*x: ([A-Za-z0-9_$]+),\s*y: ([A-Za-z0-9_$]+)\s*\}\)"#,
    )
    .unwrap();
    let popup = popup_re
        .captures(main)
        .ok_or("native dock menu anchor changed")?;
    let popup_whole = popup[0].to_owned();
    let popup_x = popup[2].to_owned();
    let popup_y = popup[3].to_owned();

    let popup_fn = function_for(&main_meta.functions, source, "\"popup_main_dock_context_menu\"")?;
    let popup_source = &source[popup_fn.start..popup_fn.end];
    let cancel_re =
        Regex::new(r"finally\s*\{[\s\S]*?,\s*([A-Za-z0-9_$]+)\(\),\s*[A-Za-z0-9_$]+ && window\.setTimeout")
            .unwrap();
    let cancel_gesture = cancel_re
        .captures(popup_source)
        .ok_or("dock cleanup binding changed")?[1]
        .to_owned();
    let ptr_re = Regex::new(r"cancelDockPointerGesture:\s*([A-Za-z0-9_$]+)").unwrap();
    let ptr_matches: Vec<String> = ptr_re
        .captures_iter(source)
        .map(|c| c[1].to_owned())
        .collect();
    if !ptr_matches.contains(&cancel_gesture) {
        return Err("dock cleanup binding changed (cancelDockPointerGesture mismatch)".into());
    }

    let swallow_fn = function_for(&main_meta.functions, source, "\"set_main_swallow_mouse\"")?;
    let swallow_source = &source[swallow_fn.start..swallow_fn.end];
    let swallow_re = Regex::new(r"function ([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\) \{").unwrap();
    let swallow_caps = swallow_re
        .captures(swallow_source)
        .ok_or("mouse interception binding changed")?;
    let swallow_whole = swallow_caps[0].to_owned();
    let swallow_arg = swallow_caps[2].to_owned();

    let mut rec = PatchRecorder::default();
    let main = rec.replace(
        main,
        &swallow_whole,
        &format!("{swallow_whole}\n if(__lg.contextMenu?.isOpen()) {swallow_arg}=false;"),
    )?;
    let native_invoke = mi
        .get("invoke$1")
        .or_else(|| mi.get("invoke"))
        .map(String::as_str)
        .ok_or("Windows context menu 缺少 invoke 导入")?;
    let context_actions = format!(
        "{{openTab:{open_tab},openConsole:async()=>{{await __lg.assistant.openPanel();}},before:async()=>{{await {cancel_gesture}();await {native_invoke}('set_main_swallow_mouse',{{enabled:false}});}},after:async()=>{{await {native_invoke}('set_main_swallow_mouse',{{enabled:false}});await {cancel_gesture}();}}}}"
    );
    let main = rec.replace(
        &main,
        &popup_whole,
        &format!(
            "try{{await __lg.contextMenu.popup({popup_x},{popup_y},{context_actions});}}\n            catch(error){{console.warn('[context-menu] native extension unavailable',error);{popup_whole};}}"
        ),
    )?;
    Ok((main, WindowsContextMenuPatch { context_actions }))
}

fn patch_windows_monitor_autostart(monitor: &str) -> String {
    let mut out = monitor.to_owned();
    let status_re = Regex::new(
        r#"return [A-Za-z0-9_$]+\(\) \? await [A-Za-z0-9_$]+\("is_elevated_autostart_enabled"\) : await [A-Za-z0-9_$]+\(\)"#,
    )
    .unwrap();
    if let Some(m) = status_re.find(&out) {
        let rep = r#"return (await window.__WUWA_LOCAL_ATLAS__.request("/startup",{action:"status"})).enabled"#;
        out.replace_range(m.start()..m.end(), rep);
    } else {
        eprintln!("warn: 官方开机自启状态查询锚点变了，设置里的开关仍走官方实现");
    }

    let set_re =
        Regex::new(r"(async function ([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\) \{)\s*try \{\s*if \([A-Za-z0-9_$]+\(\)\)")
            .unwrap();
    if let Some(caps) = set_re.captures(&out) {
        let header = caps.get(1).unwrap();
        let arg = &caps[3];
        let insert_at = header.end();
        let snippet = format!(
            "\n return window.__WUWA_LOCAL_ATLAS__.request(\"/startup\",{{action:{arg}?\"enable\":\"disable\"}});"
        );
        out.insert_str(insert_at, &snippet);
    } else {
        eprintln!("warn: 官方开机自启开关锚点变了，设置里的开关仍走官方实现");
    }
    out
}

fn read_normalized_js(path: &Path) -> Result<String, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("读取 JS 失败 ({}): {e}", path.display()))?;
    if raw.contains("\r\n") {
        Ok(raw.replace("\r\n", "\n"))
    } else {
        Ok(raw)
    }
}

pub struct IntegrationResult {
    pub manifest: Value,
    pub patch_log: Vec<PatchLogEntry>,
    pub shared_name: String,
    pub main_name: String,
    pub panel_name: String,
    pub extra_patched_assets: Vec<String>,
    pub base_dir: PathBuf,
    pub frontend_dir: PathBuf,
}

/// Execute the complete official frontend integration pipeline for either macOS or Windows.
pub fn integrate_build(
    platform: Platform,
    build_root: &Path,
    repo_root: &Path,
) -> Result<IntegrationResult, String> {
    let base_text = fs::read_to_string(build_root.join("current_base.txt"))
        .map_err(|e| format!("读取 current_base.txt 失败：{e}"))?;
    let base_dir = PathBuf::from(base_text.trim().trim_start_matches('\u{feff}'));
    let structure_raw = fs::read_to_string(base_dir.join("js_structure.json"))
        .map_err(|e| format!("读取 js_structure.json 失败：{e}"))?;
    let mut structure: JsStructure = serde_json::from_str(&structure_raw)
        .map_err(|e| format!("解析 js_structure.json 失败：{e}"))?;

    let config_raw = fs::read_to_string(build_root.join("local_config.json"))
        .map_err(|e| format!("读取 local_config.json 失败：{e}"))?;
    let config: Value = serde_json::from_str(config_raw.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("解析 local_config.json 失败：{e}"))?;

    let shared_name = structure.shared.clone();
    let readable_assets = base_dir.join("readable_js/assets");
    let shared_orig = read_normalized_js(&readable_assets.join(&shared_name))?;

    let router_pos = shared_orig
        .find("const router = createRouter")
        .ok_or("shared 模块缺少 const router = createRouter")?;
    let route_region = &shared_orig[router_pos..];
    let route_re =
        Regex::new(r#"(?s)path:\s*"([^"]+)"[\s\S]*?import\("\./([^"]+)"\)"#).unwrap();
    let mut routes = BTreeMap::new();
    for caps in route_re.captures_iter(route_region).take(9) {
        routes.insert(caps[1].to_owned(), caps[2].to_owned());
    }
    let main_name = routes
        .get("/")
        .cloned()
        .ok_or("路由表缺少 '/' 入口模块")?;
    let panel_name = routes
        .get("/panel")
        .cloned()
        .ok_or("路由表缺少 '/panel' 入口模块")?;

    let main_orig = read_normalized_js(&readable_assets.join(&main_name))?;
    let panel_orig = read_normalized_js(&readable_assets.join(&panel_name))?;

    // Convert inspect_js char offsets to UTF-8 byte offsets for main and panel before slicing.
    let main_map = char_to_byte_map(&main_orig);
    let panel_map = char_to_byte_map(&panel_orig);
    if let Some(m) = structure.files.get_mut(&main_name) {
        m.convert_offsets_to_bytes(&main_map)?;
    }
    if let Some(p) = structure.files.get_mut(&panel_name) {
        p.convert_offsets_to_bytes(&panel_map)?;
    }

    let frontend_dir = build_root.join("frontend_v2");
    if frontend_dir.exists() {
        fs::remove_dir_all(&frontend_dir).map_err(|e| e.to_string())?;
    }
    copy_dir_recursive(&base_dir.join("frontend"), &frontend_dir)?;

    let mut rec = PatchRecorder::default();

    // Aligned cross-platform order: prepare_shared -> mod_display_name -> rest.
    let mut shared = match platform {
        Platform::Mac => prepare_mac_shared(&mut rec, &shared_orig)?,
        Platform::Windows => shared_orig.clone(),
    };
    shared = patch_mod_display_name(&mut rec, &shared)?;
    shared = patch_taskbar_default(&mut rec, &shared)?;
    shared = patch_mod_asset_url(&mut rec, &shared)?;
    shared = rec.replace(
        &shared,
        "const BAG_RARITY_SORT_ORDER = [\"epic\", \"rare\", \"uncommon\", \"common\"];",
        "const BAG_RARITY_SORT_ORDER = [\"mod\", \"epic\", \"rare\", \"uncommon\", \"common\"];",
    )?;
    shared = patch_online_services(&mut rec, &shared)?;

    let pkg_json: Value = serde_json::from_str(
        &fs::read_to_string(repo_root.join("package.json"))
            .map_err(|e| format!("读取 package.json 失败：{e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let community_version = config
        .get("community_version")
        .and_then(Value::as_str)
        .or_else(|| config.get("mod_version").and_then(Value::as_str))
        .or_else(|| pkg_json.get("version").and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_owned();
    let official_version = config
        .get("client_version")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    if official_version.is_empty() {
        return Err("Official client version missing from app config".into());
    }
    shared = patch_display_version(&shared, &community_version, &official_version)?;
    shared = patch_invoke_guard(&mut rec, &shared, true)?;
    shared = patch_atlas_guards(&mut rec, &shared, "")?;

    let controller = render_frontend_module(repo_root, "local_atlas_controller.js", platform)?;
    let init = controller_init(platform, &config, &controller)?;
    shared = rec.replace(
        &shared,
        "bootstrap();\nexport {",
        &format!("{init}\nexport {{ localAtlasController as zzLocalAtlas,\n"),
    )?;

    // Main renderer edits
    let m_raw = structure
        .files
        .get(&main_name)
        .ok_or_else(|| format!("js_structure 缺少 {main_name}"))?
        .clone();
    let mut mi = m_raw.imports.clone();
    for canonical in ["listen", "invoke", "watch", "onMounted", "onUnmounted"] {
        with_alias(&mut mi, canonical);
    }
    if !mi.contains_key("invoke$1") {
        if let Some(inv) = mi.get("invoke").cloned() {
            mi.insert("invoke$1".into(), inv);
        }
    }

    let main_component = find_component(&m_raw, "index")?;
    let component_functions: Vec<FunctionMeta> = m_raw
        .functions
        .iter()
        .filter(|fn_meta| {
            fn_meta.start > main_component.body && fn_meta.start < main_component.body_end
        })
        .cloned()
        .collect();
    if m_raw.snapshots.len() != 1 {
        return Err(format!(
            "主渲染器 snapshot 数量异常：{}（期望 1）",
            m_raw.snapshots.len()
        ));
    }
    let snapshot = &m_raw.snapshots[0];
    let f = &snapshot.fields;
    let req_field = |k: &str| -> Result<&str, String> {
        f.get(k)
            .map(String::as_str)
            .ok_or_else(|| format!("snapshot 缺少字段：{k}"))
    };

    let store = require_import(&mi, "globalStore")?;
    let watch_sym = require_import(&mi, "watch")?;
    let computed_sym = require_import(&mi, "computed")?;
    let show_toast_sym = require_import(&mi, "showToast")?;
    let appear_catalog_sym = require_import(&mi, "appearanceAssetCatalog")?;
    let getchar_sym = require_import(&mi, "getUserCharacterAssetName")?;
    let getaccess_sym = require_import(&mi, "getUserAccessoryAssetName")?;
    let getaccess_id_sym = require_import(&mi, "getUserAccessoryAssetId")?;
    let getemotes_sym = require_import(&mi, "getUserEmoteAssets")?;
    let role = req_field("selectedRoleId")?;
    let accessory = req_field("selectedPartId")?;
    let host = req_field("assetsCdnHost")?;
    let bag = req_field("bagList")?;
    let atlas = req_field("atlasList")?;
    let preset_list_field = req_field("presetList")?;
    let preset_loading_field = req_field("presetListLoading")?;
    let user_info_field = req_field("userInfo")?;
    let selected_emotes_field = req_field("selectedEmoteAssetIds")?;

    let emote_re = Regex::new(&format!(
        r"(?s){}\(\(\) => {}\.isLoggedIn \? {}\({}\.userInfo\)[\s\S]{{0,250}}?\}}\)\) : ([A-Za-z0-9_$]+)\.value\)",
        regex::escape(computed_sym),
        regex::escape(store),
        regex::escape(getemotes_sym),
        regex::escape(store)
    ))
    .unwrap();
    let emotes_cap = emote_re
        .captures(&main_orig)
        .ok_or("Emote selection computed layout changed")?;
    let emotes = format!("{}.value", &emotes_cap[1]);

    let mut edits = vec![Edit::new(
        0,
        0,
        format!("import {{zzLocalAtlas as __lg}} from \"./{shared_name}\";\n"),
    )];
    let insert_fn = |edits: &mut Vec<Edit>, name: &str, code: String| -> Result<(), String> {
        let pos = named(&component_functions, name)?.body + 1;
        edits.push(Edit::new(pos, pos, format!("\n{code}\n")));
        Ok(())
    };

    let get_bag_list = require_import(&mi, "getBagList")?;
    let activity_info_imp = require_import(&mi, "activityInfo")?;
    let get_preset_list = require_import(&mi, "getPresetList")?;

    let bag_loader = function_for(&component_functions, &main_orig, &format!("await {get_bag_list}()"))?;
    let activity_loader =
        function_for(&component_functions, &main_orig, &format!("await {activity_info_imp}()"))?;

    insert_fn(&mut edits, &activity_loader.name, "try {".into())?;
    edits.push(Edit::new(
        activity_loader.body_end - 1,
        activity_loader.body_end - 1,
        "}finally{if(__lg.enabled.value)__localRefresh();}",
    ));

    let param_of = |fn_name: &str| -> Result<Option<String>, String> {
        fn_param_in_scope(&main_orig, &component_functions, &m_raw.functions, fn_name)
    };

    let select_name = m_raw
        .cases
        .get("select-appearance")
        .ok_or("缺少 case select-appearance")?;
    let select_param = param_of(select_name)?.ok_or("select-appearance 缺少参数")?;
    let select_fn = named(&component_functions, select_name)?;
    let select_source = &main_orig[select_fn.start..select_fn.end];
    let max_emotes = Regex::new(r"\.length >= ([A-Za-z0-9_$]+) \|\|")
        .unwrap()
        .captures(select_source)
        .ok_or("Emote capacity guard changed")?[1]
        .to_owned();
    let reorder_name = m_raw
        .cases
        .get("reorder-emotes")
        .ok_or("缺少 case reorder-emotes")?
        .clone();
    let reorder_param = param_of(&reorder_name)?.ok_or("reorder-emotes 缺少参数")?;

    insert_fn(
        &mut edits,
        select_name,
        format!(
            "if(!__skipLocalSelectChoice){{
 const __applyOfficialItem=async __item=>{{
  __skipLocalSelectChoice=true;
  __officialApplyDepth++;
  try{{
   const __off={store}.userInfo;
   let __req=__item&&typeof __item==='object'?{{...__item}}:__item;
   if(__off){{
    const __offChar={getchar_sym}(__off);
    {role}=__offChar?__offChar.replace(/\\.png$/i,''):'';
    {accessory}={getaccess_sym}(__off)||null;
   }}
   if(__req?.__wuwaRemove&&__req.__wuwaTarget==='official'){{
    if(__req.asset_type==='accessory'){{
     const __offAccId=__off?({getaccess_id_sym}(__off)||Number(__off?.accessory?.id)||0):0;
     const __offAccName=__off?({getaccess_sym}(__off)||__off?.accessory?.name||''):({accessory}||'');
     __req.asset_id=Number(__req.asset_id)||__offAccId;
     __req.asset_name=__req.asset_name||__offAccName||(__req.asset_id?'accessory_'+__req.asset_id:'');
     if(__req.asset_name){accessory}=__req.asset_name;
    }}else if(__req.asset_type==='emote'){{
     const __list=__off?{getemotes_sym}(__off):{emotes};
     const __idx=Number(__req.__wuwaSlotIndex);
     const __hit=(Array.isArray(__list)?__list:[]).find(x=>Number(x?.id)===Number(__req.asset_id))||(Number.isInteger(__idx)&&Array.isArray(__list)?__list[__idx]:null);
     if(__hit?.id){{__req.asset_id=Number(__hit.id);__req.asset_name=__req.asset_name||__hit.name||('emote_'+__hit.id);}}
    }}
   }}
   return await {select_name}(__req);
  }}finally{{
   __skipLocalSelectChoice=false;
   __officialApplyDepth--;
   __localRefresh();
  }}
 }};
 const __isModItem=Boolean(__lg.isModAsset?.({select_param})||__lg.isModAsset?.({select_param}?.asset_id));
 if(__isModItem&&!{select_param}?.__wuwaReorder){{
  if({select_param}?.__wuwaRemove){{
   if({select_param}?.__wuwaTarget!=='official'){{__lg.beginSelect({{...{select_param},__wuwaTarget:'local'}},{store}.userInfo,{max_emotes},null,{{noDialog:true}});__localRefresh();}}
   return;
  }}
  try{{
   if(!__lg.enabled.value)await (__lg.activateLocalIfReady?.()||__lg.startOverlay?.());
   __lg.select({select_param},{store}.userInfo,{max_emotes},{select_param}?.__wuwaTarget==='local'?{{forceEquip:true,slotIndex:{select_param}.__wuwaSlotIndex}}:{{}});
   __localRefresh();
  }}catch(error){{{show_toast_sym}(error.message);}}
  return;
 }}
 if({select_param}?.wuwa_choice){{
  const __choice={select_param}.wuwa_choice;
  if(__choice==='local'){{
   try{{
    if(!__lg.enabled.value)await (__lg.activateLocalIfReady?.()||__lg.startOverlay?.());
    __lg.select({select_param},{store}.userInfo,{max_emotes});
    __localRefresh();
   }}catch(error){{{show_toast_sym}(error.message);}}
   return;
  }}
  if(__isModItem||{select_param}.owned===false)return;
  await __applyOfficialItem({select_param});
  __lg.setMark({select_param}.asset_id,{select_param}.asset_type,'official',true);
  return;
 }}
 if(__lg.enabled.value || {select_param}?.__wuwaReorder || {select_param}?.__wuwaTarget){{
  if({select_param}?.__wuwaReorder){{
   const __ids=Array.isArray({select_param}.emoteAssetIds)?{select_param}.emoteAssetIds:[];
   if({select_param}.__wuwaTarget==='official'){{
    __skipLocalReorder=true;__officialApplyDepth++;
    Promise.resolve({reorder_name}(__ids)).finally(()=>{{__officialApplyDepth--;__skipLocalReorder=false;__localRefresh();}}).catch(err=>console.error('[reorder-official]',err));
   }}else{{
    __lg.reorder(__ids,{store}.userInfo);__localRefresh();
   }}
   return;
  }}
  if(__lg.beginSelect({select_param},{store}.userInfo,{max_emotes},()=>__applyOfficialItem({select_param}),{{noDialog:true}})){{
   __localRefresh();
   return;
  }}
 }}
}}"
        ),
    )?;

    insert_fn(
        &mut edits,
        &reorder_name,
        format!(
            "if(!__skipLocalReorder && ((__lg.enabled.value && Array.isArray(__lg.selection.value?.emote)) || (Array.isArray({reorder_param}) && {reorder_param}.some(__id=>__lg.isModAsset?.(__id))))){{if(__lg.enabled.value)__lg.reorder({reorder_param},{store}.userInfo);__localRefresh();return;}}"
        ),
    )?;

    let favorite_name = m_raw
        .cases
        .get("collect-change")
        .ok_or("缺少 case collect-change")?;
    let favorite_param = param_of(favorite_name)?.ok_or("collect-change 缺少参数")?;
    insert_fn(
        &mut edits,
        favorite_name,
        format!(
            "if(__lg.isModAsset?.({favorite_param}.assetId)){{__lg.favorite({favorite_param}.assetId,{favorite_param}.is_collect===1);return;}}"
        ),
    )?;

    let preset_loader =
        function_for(&component_functions, &main_orig, &format!("await {get_preset_list}()"))?;
    let preset_source = &main_orig[preset_loader.start..preset_loader.end];
    let preset_normalize_re = Regex::new(&format!(
        r"{} = [^\n]+\.map\(([A-Za-z0-9_$]+)\)",
        regex::escape(preset_list_field)
    ))
    .unwrap();
    let preset_normalize = preset_normalize_re
        .captures(preset_source)
        .ok_or("Preset list normalization changed")?[1]
        .to_owned();
    let create_preset_fn = named(
        &component_functions,
        m_raw
            .cases
            .get("create-preset")
            .ok_or("缺少 case create-preset")?,
    )?;
    let preset_limit_re =
        Regex::new(&format!(r"{}\.length >= ([A-Za-z0-9_$]+)", regex::escape(preset_list_field)))
            .unwrap();
    let preset_limit = preset_limit_re
        .captures(&main_orig[create_preset_fn.start..create_preset_fn.end])
        .ok_or("Preset capacity guard changed")?[1]
        .to_owned();

    let create_op = format!("__lg.createLocalPreset({store}.userInfo,{preset_limit})");
    for (action, template) in [
        ("create-preset", create_op.as_str()),
        ("delete-preset", "__lg.deleteLocalPreset(Number(__pid))"),
        ("apply-preset", "__lg.applyLocalPreset(Number(__pid))"),
    ] {
        let action_name = m_raw
            .cases
            .get(action)
            .ok_or_else(|| format!("缺少 case {action}"))?;
        let arg = param_of(action_name)?.unwrap_or_else(|| "null".into());
        let op = template.replace("{arg}", &arg);
        let guard = match action {
            "create-preset" => format!(
                "if(__lg.enabled.value && __lg.shouldCreateLocalPreset({store}.userInfo)){{try{{{op};__localRefresh();}}catch(error){{{show_toast_sym}(error.message);}}return;}}"
            ),
            _ => format!(
                "const __pid=typeof {arg}==='object'&&{arg}!==null?{arg}.preset_id:{arg};\n\
                 if(Number(__pid)<0){{if(__lg.enabled.value){{try{{{op};__localRefresh();}}catch(error){{{show_toast_sym}(error.message);}}}}return;}}"
            ),
        };
        insert_fn(&mut edits, action_name, guard)?;
    }

    let snapshot_fn = function_for(&component_functions, &main_orig, "streamId:")?
        .name
        .clone();
    let snap_push_re = Regex::new(&format!(
        r"function ([A-Za-z0-9_$]+)\(\) \{{\s*[A-Za-z0-9_$]+\.value && [A-Za-z0-9_$]+\({}\(\)\)",
        regex::escape(&snapshot_fn)
    ))
    .unwrap();
    let snap_push_fn = snap_push_re
        .captures(&main_orig)
        .ok_or("snapshot push helper changed")?[1]
        .to_owned();

    let mut snap_text = main_orig[snapshot.start..snapshot.end].to_owned();
    snap_text = snap_text.replace(
        &format!("bagList: {bag}"),
        &format!("bagList: __lg.inventory({atlas},{bag})"),
    );
    snap_text = snap_text.replace(
        &format!("atlasList: {atlas}"),
        &format!("atlasList: __lg.enabled.value ? __lg.mergeAtlas({atlas}) : {atlas}"),
    );
    snap_text = snap_text.replace(
        &format!("presetList: {preset_list_field}"),
        &format!(
            "presetList: __lg.enabled.value ? __lg.mergePresets(__lg.presetList().map({preset_normalize}), {preset_list_field}) : {preset_list_field}"
        ),
    );
    snap_text = snap_text.replace(
        &format!("presetListLoading: {preset_loading_field}"),
        &format!("presetListLoading: {preset_loading_field}"),
    );
    let rewritten_user_info = user_info_field.replace(
        &format!("{store}.userInfo"),
        &format!("__lg.viewUser({store}.userInfo)"),
    );
    snap_text = rec.replace(
        &snap_text,
        &format!("userInfo: {user_info_field}"),
        &format!(
            "userInfo: {rewritten_user_info},\n          officialUserInfo: {user_info_field},\n          localSelection: __lg.selection.value"
        ),
    )?;
    snap_text = snap_text.replace(
        &format!("selectedEmoteAssetIds: {selected_emotes_field}"),
        &format!(
            "selectedEmoteAssetIds: {getemotes_sym}(__lg.viewUser({store}.userInfo)).map(item=>item.id)"
        ),
    );
    edits.push(Edit::new(snapshot.start, snapshot.end, snap_text));

    for meaning in [
        "getUserCharacterAssetId",
        "getUserCharacterAssetName",
        "getUserAccessoryAssetId",
        "getUserAccessoryAssetName",
        "getUserEmoteAssets",
        "getUserAccessoryOffset",
        "getUserCharacterMyOffset",
    ] {
        let alias = require_import(&mi, meaning)?;
        let call_re =
            Regex::new(&format!(r"{}\({}\.userInfo\)", regex::escape(alias), regex::escape(store)))
                .unwrap();
        for m in call_re.find_iter(&main_orig) {
            if !(snapshot.start <= m.start() && m.start() < snapshot.end) {
                edits.push(Edit::new(
                    m.start(),
                    m.end(),
                    format!("{alias}(__effectiveUser({store}.userInfo))"),
                ));
            }
        }
    }

    let local_code = format!(
        r#"
 let __localSaved=null,__skipLocalSelectChoice=false,__skipLocalReorder=false,__officialApplyDepth=0;
function __effectiveUser(u){{return __officialApplyDepth>0?u:__lg.viewUser(u);}}
function __localRefresh(){{
 if(!__lg.enabled.value||__officialApplyDepth>0)return;
 const user=__lg.viewUser({store}.userInfo);
 {host}=__lg.assetsHost;
 const name={getchar_sym}(user);if(name){role}=name.replace(/\.png$/i,'');
 {accessory}={getaccess_sym}(user)||null;
 {emotes}={getemotes_sym}(user).map(item=>({{id:item.id,name:item.name}}));
 {atlas}=__lg.mergeAtlas({atlas});
 {snap_push_fn}();
}}
{watch_sym}(__lg.enabled,on=>{{
 if(on){{__localSaved={{role:{role},accessory:{accessory},emotes:[...{emotes}],host:{host},atlas:{atlas}}};__localRefresh();}}
 else if(__localSaved){{{role}=__localSaved.role;{accessory}=__localSaved.accessory;{emotes}=__localSaved.emotes;{host}=__localSaved.host;{atlas}=__localSaved.atlas;__localSaved=null;{act_fn}({{force:true}}).then(()=>{{{bag_fn}()}}).catch(()=>{{}});}}
}},{{immediate:true}});
{watch_sym}([__lg.selection,__lg.revision,{appear_catalog_sym}],()=>{{if(__lg.enabled.value)__localRefresh();}});
{watch_sym}([()=>{role},()=>{accessory},()=>{host}],()=>{{if(__lg.enabled.value)__localRefresh();}});
"#,
        act_fn = activity_loader.name,
        bag_fn = bag_loader.name,
    );
    let main_return = main_component
        .return_start
        .ok_or("主组件缺少 returnStart")?;
    edits.push(Edit::new(main_return, main_return, local_code));
    let mut main = apply_edits(&main_orig, edits)?;
    let accessory_alias = require_import(&mi, "getUserAccessoryOffset")?;
    main = patch_accessory_transform(&mut rec, &main, accessory_alias)?;
    main = patch_emotion_menu_raf(&mut rec, &main, watch_sym)?;
    let on_unmounted_m = require_import(&mi, "onUnmounted")?;
    main = patch_room_layout_hooks(&mut rec, &main, on_unmounted_m)?;
    main = patch_sync_emotion(&mut rec, &main)?;
    // Panel edits
    let p_raw = structure
        .files
        .get(&panel_name)
        .ok_or_else(|| format!("js_structure 缺少 {panel_name}"))?;
    let mut pi = p_raw.imports.clone();
    for canonical in ["listen", "invoke", "watch", "onMounted", "onUnmounted"] {
        with_alias(&mut pi, canonical);
    }
    let settings = find_component(p_raw, "SettingsModule")?;
    let atlas_component = find_component(p_raw, "MainPanelAtlasTab")?;
    let container = p_raw
        .containers
        .iter()
        .find(|c| settings.body < c.start && c.start < settings.body_end)
        .ok_or("SettingsModule 缺少 panel-content 容器")?;
    let h = require_import(&pi, "createBaseVNode")?;
    let with_modifiers = require_import(&pi, "withModifiers")?;
    let show_toast_p = require_import(&pi, "showToast")?;

    let setup = format!(
        "const __localBusy=__lg.busy,__localSync=__lg.syncStatus,__localCache=__lg.cacheReady;\n\
async function __syncLocal(){{try{{const status=await __lg.syncCache(true);{show_toast_p}('素材缓存已同步：'+(status.result?.atlas_count||'')+'项');}}catch(error){{{show_toast_p}(error.message);}}}}\n\
function __openCommunityUpdate(event){{try{{event&&event.stopPropagation&&event.stopPropagation();__lg.assistant.openPanel('update');}}catch(error){{console.error('[assistant]',error);}}}}\n"
    );
    let row = format!(
        "{h}('div',{{class:'setting-item','data-local-gallery-setting':'true'}},[\n\
 {h}('div',{{class:'setting-label'}},[{h}('div',{{class:'setting-title'}},'外观与挂饰调节'),{h}('div',{{class:'setting-sub'}},'官方图鉴、背包和联机保持原样；本地资源只补充本机显示。',1)]),\n\
 {h}('div',{{class:'setting-content'}},[{h}('button',{{class:'setting-btn',type:'button',style:'white-space:nowrap',onMousedown:{with_modifiers}(()=>{{}},['stop']),onClick:()=>__lg.assistant.openPanel('mods')}},'外观与挂饰')])]),\n\
 {h}('div',{{class:'setting-item','data-local-gallery-sync':'true'}},[{h}('div',{{class:'setting-label'}},[{h}('div',{{class:'setting-title'}},'同步素材缓存'),{h}('div',{{class:'setting-sub'}},__localSync.value.status==='running'?'正在同步 '+__localSync.value.done+'/'+__localSync.value.total:!__localCache.value?'首次启动需要联网补齐素材缓存':'本机素材缓存已就绪',1)]),{h}('div',{{class:'setting-content'}},[{h}('button',{{class:'setting-btn',type:'button',disabled:__localBusy.value||__localSync.value.status==='running',onClick:__syncLocal}},'检查并同步')])]),\n\
");
    let mut panel_edits = vec![
        Edit::new(
            0,
            0,
            format!("import {{zzLocalAtlas as __lg}} from \"./{shared_name}\";\n"),
        ),
        Edit::new(settings.body + 1, settings.body + 1, setup),
        Edit::new(container.array + 1, container.array + 1, row),
    ];

    // Native Console tab in panel (extend_panel)
    let module_component = find_component(p_raw, "MainModulePanel")?;
    let module_text = &panel_orig[module_component.start..module_component.end];
    let nav_iter =
        Regex::new(r"([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\), ([A-Za-z0-9_$]+) =>")
            .unwrap()
            .captures(module_text)
            .ok_or("navigation iterator changed")?;
    let (nav_render, nav_unref, nav_items, nav_item) =
        (&nav_iter[1], &nav_iter[2], &nav_iter[3], &nav_iter[4]);
    let nav_state = Regex::new(r#"([A-Za-z0-9_$]+)\.activeTab === "bag""#)
        .unwrap()
        .captures(module_text)
        .ok_or("navigation state binding changed")?[1]
        .to_owned();
    let nav_label = Regex::new(r#"return ([A-Za-z0-9_$]+)\.key === "toggle""#)
        .unwrap()
        .captures(module_text)
        .ok_or("navigation label binding changed")?[1]
        .to_owned();

    let define_component = require_import(&pi, "defineComponent")?;
    let ref_p = require_import(&pi, "ref")?;
    let on_mounted_p = require_import(&pi, "onMounted")?;
    let on_unmounted_p = require_import(&pi, "onUnmounted")?;
    let computed_p = require_import(&pi, "computed")?;
    let watch_p = require_import(&pi, "watch")?;
    let unref_p = require_import(&pi, "unref")?;
    let open_block_p = require_import(&pi, "openBlock")?;
    let create_elem_block_p = require_import(&pi, "createElementBlock")?;
    let create_block_p = require_import(&pi, "createBlock")?;

    let console_component = format!(
        "\nconst __ConsolePage={define_component}({{\n\
 __name:'WuwaNativeConsole',setup(){{\n\
  const target={ref_p}(null);\n\
  {on_mounted_p}(()=>__lg.assistant.mountConsole(target.value));\n\
  {on_unmounted_p}(()=>__lg.assistant.closePanel());\n\
  return ()=>{h}('div',{{class:'module-block',style:{{height:'100%',minHeight:0,position:'relative',overflow:'hidden',paddingTop:'6px'}}}},[{h}('div',{{ref:target,style:{{height:'100%',minHeight:0,overflow:'auto'}}}})]);\n\
 }}\n\
}});\n\
const __consoleTabs={computed_p}(()=>{unref_p}({nav_items}).flatMap(item=>item.key==='setting'?[{{key:'console',labelKey:'console'}},item]:[item]));\n"
    );
    panel_edits.push(Edit::new(
        module_component.body + 1,
        module_component.body + 1,
        console_component,
    ));

    let icon_re = Regex::new(&format!(
        r#"{}\("span", \{{\s*class: [A-Za-z0-9_$]+\(\["tab-btn__icon", `tab-btn__icon--\$\{{{}\.key\}}`\]\)\s*\}}, null, 2\)"#,
        regex::escape(h),
        regex::escape(nav_item)
    ))
    .unwrap();
    let icon_match = icon_re
        .find(module_text)
        .ok_or("native navigation icon layout changed")?;
    let console_icon = format!(
        "{nav_item}.key==='console'?({open_block_p}(),{create_elem_block_p}('svg',{{key:'console-icon',class:'tab-btn__icon tab-btn__icon--console',viewBox:'0 0 40 54','aria-hidden':'true','data-console-icon':'sliders',style:{{color:{nav_state}.activeTab==='console'?'#9c6541':'#f4d5a5'}}}},[\n\
 {h}('rect',{{x:7,y:15,width:26,height:25,rx:5,fill:'none',stroke:'currentColor','stroke-width':2.5}}),\n\
 {h}('path',{{d:'M14 21v13M20 21v13M26 21v13',fill:'none',stroke:'currentColor','stroke-width':2.2,'stroke-linecap':'round'}}),\n\
 {h}('circle',{{cx:14,cy:25,r:2.6,fill:'currentColor'}}),{h}('circle',{{cx:20,cy:30,r:2.6,fill:'currentColor'}}),{h}('circle',{{cx:26,cy:24,r:2.6,fill:'currentColor'}})\n\
],4)):{}",
        icon_match.as_str()
    );
    panel_edits.push(Edit::new(
        module_component.start + icon_match.start(),
        module_component.start + icon_match.end(),
        console_icon,
    ));

    for (old, new) in [
        (
            format!("return {nav_label}.key === \"toggle\""),
            format!(
                "return {nav_label}.key === \"console\" ? \"控制台\" : {nav_label}.key === \"toggle\""
            ),
        ),
        (
            format!("{nav_render}({nav_unref}({nav_items}),"),
            format!("{nav_render}(__consoleTabs.value,"),
        ),
        (
            format!("{nav_state}.activeTab === \"bag\" ?"),
            format!(
                "{nav_state}.activeTab === \"console\" ? ({open_block_p}(),{create_block_p}(__ConsolePage,{{key:\"console\"}})) : {nav_state}.activeTab === \"bag\" ?"
            ),
        ),
    ] {
        if module_text.match_indices(&old).count() != 1 {
            return Err(format!("native console anchor '{old}' not unique"));
        }
        let pos = module_component.start + module_text.find(&old).unwrap();
        panel_edits.push(Edit::new(pos, pos + old.len(), new));
    }

    let module_emit = Regex::new(r#"([A-Za-z0-9_$]+)\("apply-preset","#)
        .unwrap()
        .captures(module_text)
        .ok_or("MainModulePanel apply-preset emit missing")?[1]
        .to_owned();
    let char_confirm_re =
        Regex::new(r#"if \(([A-Za-z0-9_$]+)\.asset_type === "character_skin"\) \{"#).unwrap();
    let char_confirm_matches: Vec<_> = char_confirm_re.captures_iter(module_text).collect();
    if char_confirm_matches.len() != 1 {
        return Err("MainModulePanel character confirm guard changed".into());
    }
    let cc_m = &char_confirm_matches[0];
    let cc_span = cc_m.get(0).unwrap();
    let cc_var = &cc_m[1];
    panel_edits.push(Edit::new(
        module_component.start + cc_span.start(),
        module_component.start + cc_span.end(),
        format!(
            "if ({cc_var}.asset_type === \"character_skin\" && {cc_var}.__wuwaTarget !== \"local\" && {cc_var}.wuwa_choice !== \"local\" && !{cc_var}.__wuwaRemove) {{"
        ),
    ));
    let preset_confirm_re = Regex::new(
        r#"const ([A-Za-z0-9_$]+) = [A-Za-z0-9_$]+\.bagList\.find\(([A-Za-z0-9_$]+) => ([A-Za-z0-9_$]+)\.asset_type === "character_skin" && ([A-Za-z0-9_$]+)\.asset_id === ([A-Za-z0-9_$]+)\.character_asset_id\);"#,
    )
    .unwrap();
    let pc_matches: Vec<_> = preset_confirm_re
        .captures_iter(module_text)
        .filter(|c| &c[2] == &c[3] && &c[2] == &c[4])
        .collect();
    if pc_matches.len() != 1 {
        return Err("MainModulePanel preset apply guard changed".into());
    }
    let pc_m = &pc_matches[0];
    let pc_span = pc_m.get(0).unwrap();
    let pc_param = &pc_m[5];
    panel_edits.push(Edit::new(
        module_component.start + pc_span.start(),
        module_component.start + pc_span.start(),
        format!("if (Number({pc_param}?.preset_id) < 0) {{ {module_emit}(\"apply-preset\", {pc_param}); return; }}\n        "),
    ));

    let bag_item = find_component(p_raw, "BagAtlasItem")?;
    let bag_item_text = &panel_orig[bag_item.start..bag_item.end];
    let props_var = Regex::new(r#""atlas-item--active":\s*([A-Za-z0-9_$]+)\.active"#)
        .unwrap()
        .captures(bag_item_text)
        .ok_or("BagAtlasItem active prop binding changed")?[1]
        .to_owned();
    let div_click_re = Regex::new(
        r#"(\{\s*class:\s*[A-Za-z0-9_$]+\(\["atlas-item",[\s\S]{0,400}?\]\),\s*)(onClick:\s*[A-Za-z0-9_$]+)"#,
    )
    .unwrap();
    let div_m = div_click_re
        .captures(bag_item_text)
        .ok_or("BagAtlasItem div onClick anchor changed")?;
    let div_click = div_m.get(2).unwrap();
    panel_edits.push(Edit::new(
        bag_item.start + div_click.start(),
        bag_item.start + div_click.end(),
        format!(
            "\"data-wuwa-asset-id\": {props_var}.item?.asset_id, \"data-wuwa-asset-type\": {props_var}.item?.asset_type, \"data-wuwa-asset-name\": {props_var}.item?.asset_name, {}",
            &div_m[2]
        ),
    ));
    let end_flag_rel = bag_item_text
        .rfind("], 2))")
        .filter(|pos| bag_item_text.len() - pos < 45)
        .ok_or("BagAtlasItem root patchFlag changed")?;
    panel_edits.push(Edit::new(
        bag_item.start + end_flag_rel,
        bag_item.start + end_flag_rel + "], 2))".len(),
        "], 10, [\"data-wuwa-asset-id\", \"data-wuwa-asset-type\", \"data-wuwa-asset-name\"]))",
    ));

    let panel_root = find_component(p_raw, "index")?;
    let root_source = &panel_orig[panel_root.body..panel_root.body_end];
    let state_ref_re = Regex::new(&format!(
        r"const ([A-Za-z0-9_$]+) = {}\(null\)",
        regex::escape(ref_p)
    ))
    .unwrap();
    let state_ref_name = if let Some(caps) = state_ref_re.captures(root_source) {
        caps[1].to_owned()
    } else {
        let sub_re = Regex::new(
            r"(?s)=\s*await\s+[A-Za-z0-9_$]+\(\s*([A-Za-z0-9_$]+)\s*=>\s*\{(.{0,800}?)\n\s*\}\)",
        )
        .unwrap();
        let sub_caps = sub_re
            .captures(root_source)
            .ok_or("Panel snapshot subscription changed")?;
        let param = &sub_caps[1];
        let body = &sub_caps[2];
        let got_re =
            Regex::new(&format!(r"([A-Za-z0-9_$]+)\.value = {}\b", regex::escape(param))).unwrap();
        got_re
            .captures(body)
            .ok_or("Panel snapshot state assignment changed")?[1]
            .to_owned()
    };
    let ready_retry = format!(
        "\nlet __panelDisposed=false;\n\
{watch_p}({state_ref_name},__s=>{{if(__s)__lg.syncPanelState?.(__s);}},{{immediate:true}});\n\
{on_unmounted_p}(()=>{{__panelDisposed=true;}});\n\
{on_mounted_p}(()=>{{
 for(const delay of [500,1500,3000,6000,10000])window.setTimeout(()=>{{
  if(!__panelDisposed&&!{state_ref_name}.value)window.__TAURI_INTERNALS__.invoke('plugin:event|emit',{{event:'cat-panel-action',payload:{{type:'ready'}}}}).catch(()=>{{}});
 }},delay);
}});\n"
    );
    let panel_return = panel_root
        .return_start
        .ok_or("面板根组件缺少 returnStart")?;
    panel_edits.push(Edit::new(panel_return, panel_return, ready_retry));

    let atlas_text = &panel_orig[atlas_component.start..atlas_component.end];
    let atlas_emit = Regex::new(r#"([A-Za-z0-9_$]+)\("select-appearance","#)
        .unwrap()
        .captures(atlas_text)
        .ok_or("MainPanelAtlasTab select-appearance emit missing")?[1]
        .to_owned();
    let atlas_owned_re = Regex::new(
        r"if \(!([A-Za-z0-9_$]+)\.owned \|\| !([A-Za-z0-9_$]+)\.asset_id\) return null;",
    )
    .unwrap();
    let atlas_owned_m = atlas_owned_re
        .captures_iter(atlas_text)
        .find(|c| &c[1] == &c[2])
        .ok_or("MainPanelAtlasTab owned guard changed")?;
    let ao_span = atlas_owned_m.get(0).unwrap();
    let ao_var = &atlas_owned_m[1];
    panel_edits.push(Edit::new(
        atlas_component.start + ao_span.start(),
        atlas_component.start + ao_span.end(),
        format!("if ((!<v>.owned && !__lg.enabled.value && !window.__WUWA_PENDING_CHOICE__) || !<v>.asset_id) return null;").replace("<v>", ao_var),
    ));
    let atlas_sanitize_re = Regex::new(
        r"(?s)(star_level:\s*([A-Za-z0-9_$]+)\.star_level)(\s*\}\s*:\s*null)",
    )
    .unwrap();
    if let Some(caps) = atlas_sanitize_re.captures(atlas_text) {
        let field_end = caps.get(1).unwrap().end();
        let src_var = &caps[2];
        panel_edits.push(Edit::new(
            atlas_component.start + field_end,
            atlas_component.start + field_end,
            format!(", wuwa_choice: {src_var}.wuwa_choice || window.__WUWA_PENDING_CHOICE__"),
        ));
    }
    let atlas_click_re = Regex::new(&format!(
        r#"([A-Za-z0-9_$]+) && ([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\) && {}\("select-appearance", ([A-Za-z0-9_$]+)\)"#,
        regex::escape(&atlas_emit)
    ))
    .unwrap();
    let atlas_click_m = atlas_click_re
        .captures_iter(atlas_text)
        .find(|c| &c[1] == &c[4])
        .ok_or("MainPanelAtlasTab click handler changed")?;
    let ac_span = atlas_click_m.get(0).unwrap();
    panel_edits.push(Edit::new(
        atlas_component.start + ac_span.start(),
        atlas_component.start + ac_span.end(),
        format!(
            "{w} && (window.__WUWA_PENDING_CHOICE__ === \"local\" ? Boolean({arg}?.asset_id) : (__lg.enabled.value ? __lg.canSelect({arg}) : {h_fn}({arg}))) && {atlas_emit}(\"select-appearance\", {w})",
            w = &atlas_click_m[1],
            h_fn = &atlas_click_m[2],
            arg = &atlas_click_m[3],
        ),
    ));

    let merge_check_re = Regex::new(&format!(
        r"[A-Za-z0-9_$]+ = {}\(\(\) => [A-Za-z0-9_$]+\([A-Za-z0-9_$]+\([A-Za-z0-9_$]+\.atlasList, [A-Za-z0-9_$]+\.bagList\)\)\)",
        regex::escape(computed_p)
    ))
    .unwrap();
    if !merge_check_re.is_match(atlas_text) {
        return Err("atlas row merge computed changed".into());
    }
    let category_re = Regex::new(&format!(
        r"([A-Za-z0-9_$]+) = {}\(\(\) => \{{\s*const [A-Za-z0-9_$]+ = [A-Za-z0-9_$]+\[([A-Za-z0-9_$]+)\.value\];",
        regex::escape(computed_p)
    ))
    .unwrap();
    let cat_caps = category_re
        .captures(atlas_text)
        .ok_or("atlas category computed changed")?;
    let (category_rows, category_index) = (&cat_caps[1], &cat_caps[2]);
    let atlas_loading_re = Regex::new(&format!(
        r"([A-Za-z0-9_$]+) = {}\(\(\) => [A-Za-z0-9_$]+\.bagListLoading \|\| [A-Za-z0-9_$]+\.atlasListLoading\)",
        regex::escape(computed_p)
    ))
    .unwrap();
    let atlas_loading_var = atlas_loading_re
        .captures(atlas_text)
        .ok_or("atlas loading computed changed")?[1]
        .to_owned();
    let atlas_return = atlas_component
        .return_start
        .ok_or("MainPanelAtlasTab 缺少 returnStart")?;
    let global_store_p = require_import(&pi, "globalStore")?;
    panel_edits.push(Edit::new(
        atlas_return,
        atlas_return,
        format!(
            "const __atlasProgress={computed_p}(()=>__lg.collectionProgress({category_rows}.value,{atlas_loading_var}.value)),__atlasProgressTitle={computed_p}(()=>['全图鉴收集进度','全角色收集进度','全配件收集进度','全表情收集进度'][{category_index}.value]||'全图鉴收集进度');\n\
             let __disposeAtlasEquip=null;\n\
             {on_mounted_p}(()=>{{__disposeAtlasEquip=__lg.mountEquipmentBar?.(document.body,{{page:'atlas',floating:true,emitSelect:x=>{atlas_emit}('select-appearance',x)}})||null;}});\n\
             {on_unmounted_p}(()=>{{__disposeAtlasEquip?.();__disposeAtlasEquip=null;}});\n"
        ),
    ));
    let tab_anchor = "null, 8, [\"modelValue\", \"tabs\"]),";
    if atlas_text.match_indices(tab_anchor).count() != 1 {
        return Err("atlas tab header changed".into());
    }
    let progress_ui = format!(
        " ({open_block_p}(true),{create_elem_block_p}('section',{{key:'account-atlas-progress','data-atlas-progress':'online',style:{{padding:'12px 4px 14px',width:'100%',boxSizing:'border-box'}}}},[\n\
 {h}('div',{{style:{{display:'flex',justifyContent:'space-between',alignItems:'center',gap:'12px',fontSize:'14px',color:'#927b65',marginBottom:'8px'}}}},[\n\
 {h}('span',null,__atlasProgressTitle.value),{h}('strong',{{style:{{fontVariantNumeric:'tabular-nums',color:'#996644'}}}},\n\
 !{global_store_p}.isLoggedIn?'登录后查看':{atlas_loading_var}.value?'正在读取…':__atlasProgress.value.collected+'/'+__atlasProgress.value.total+' · '+__atlasProgress.value.percent.toFixed(1)+'%')]),\n\
 {h}('div',{{role:'progressbar','aria-label':'真实账号'+__atlasProgressTitle.value,'aria-valuemin':0,'aria-valuemax':__atlasProgress.value.total,'aria-valuenow':__atlasProgress.value.collected,'aria-busy':{atlas_loading_var}.value,style:{{height:'9px',width:'100%',background:'#ecddc4',borderRadius:'20px',overflow:'hidden'}}}},[\n\
 {h}('span',{{style:{{display:'block',height:'100%',width:({global_store_p}.isLoggedIn?__atlasProgress.value.percent:0)+'%',background:'linear-gradient(90deg,#e4b56f,#c58b51)',borderRadius:'inherit',transition:'width .25s ease'}}}})])],-2)),"
    );
    let progress_at = atlas_component.start + atlas_text.find(tab_anchor).unwrap() + tab_anchor.len();
    panel_edits.push(Edit::new(progress_at, progress_at, progress_ui));

    let bag_merge_re = Regex::new(
        r"\(\s*([A-Za-z0-9_$]+)\(\s*([A-Za-z0-9_$]+)\.atlasList\s*,\s*([A-Za-z0-9_$]+)\.bagList\s*\)\s*\)",
    )
    .unwrap();
    if let Some(m) = bag_merge_re.captures(&panel_orig) {
        let span = m.get(0).unwrap();
        panel_edits.push(Edit::new(
            span.start(),
            span.end(),
            format!(
                "({}({}.atlasList, window.__WUWA_LOCAL_ATLAS__?.ownedBag?.() || {}.bagList))",
                &m[1], &m[2], &m[3]
            ),
        ));
    } else {
        eprintln!("warn: 找不到图鉴与背包的合并点，跳过「未拥有角标」");
    }

    let clickable_re = Regex::new(r#""is-item-clickable":\s*([A-Za-z0-9_$]+)"#).unwrap();
    for caps in clickable_re.captures_iter(&panel_orig) {
        let m = caps.get(0).unwrap();
        if panel_orig.as_bytes().get(m.end()) == Some(&b'.') {
            continue;
        }
        let name = &caps[1];
        panel_edits.push(Edit::new(
            m.start(),
            m.end(),
            format!(
                "\"is-item-clickable\": (__c => window.__WUWA_LOCAL_ATLAS__ ? Boolean(__c?.asset_id) : {name}(__c))"
            ),
        ));
    }

    let preset_add_re = Regex::new(
        r#"\{\s*"preset-item--add":\s*([A-Za-z0-9_$]+)\.kind\s*===\s*"add"\s*\}"#,
    )
    .unwrap();
    for caps in preset_add_re.captures_iter(&panel_orig) {
        let m = caps.get(0).unwrap();
        let item = &caps[1];
        panel_edits.push(Edit::new(
            m.start(),
            m.end(),
            format!(
                "{{\"preset-item--add\": {item}.kind === \"add\", \"preset-item--account\": {item}.kind === \"preset\" && ({item}.preset?.preset_id ?? 0) > 0, \"preset-item--local\": {item}.kind === \"preset\" && ({item}.preset?.preset_id ?? 0) <= 0}}"
            ),
        ));
    }

    let bag_component = find_component(p_raw, "MainPanelBagTab")?;
    let bag_text = &panel_orig[bag_component.start..bag_component.end];
    let panel_select_re = Regex::new(r"onSelectItem\s*:\s*([A-Za-z0-9_$]+)\s*,").unwrap();
    let mut wrapped_panel_select = 0usize;
    for (range, force_official) in [
        (atlas_component.start..atlas_component.end, false),
        (bag_component.start..bag_component.end, true),
    ] {
        for caps in panel_select_re.captures_iter(&panel_orig[range.clone()]) {
            let m = caps.get(0).unwrap();
            let handler = &caps[1];
            wrapped_panel_select += 1;
            panel_edits.push(Edit::new(
                range.start + m.start(),
                range.start + m.end(),
                format!(
                    "onSelectItem: (__c => {{ const __lg=window.__WUWA_LOCAL_ATLAS__; if(__lg?.beginPanelSelect) return __lg.beginPanelSelect(__c, __next=>{{ window.__WUWA_PENDING_CHOICE__=__next.wuwa_choice; try{{return {handler}(__next);}}finally{{window.__WUWA_PENDING_CHOICE__=null;}} }}, {force_official}); return {handler}(__c); }}),"
                ),
            ));
        }
    }
    if wrapped_panel_select != 2 {
        return Err(format!("MainPanelAtlasTab/BagTab onSelectItem anchor count changed: {wrapped_panel_select}"));
    }
    let bag_emit = Regex::new(r#"([A-Za-z0-9_$]+)\("select-appearance","#)
        .unwrap()
        .captures(bag_text)
        .ok_or("MainPanelBagTab select-appearance emit missing")?[1]
        .to_owned();
    let bag_click_re = Regex::new(&format!(
        r#"([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\) && {}\("select-appearance", ([A-Za-z0-9_$]+)\)"#,
        regex::escape(&bag_emit)
    ))
    .unwrap();
    let bag_click_m = bag_click_re
        .captures_iter(bag_text)
        .find(|c| &c[2] == &c[3])
        .ok_or("MainPanelBagTab click handler changed")?;
    let bc_span = bag_click_m.get(0).unwrap();
    panel_edits.push(Edit::new(
        bag_component.start + bc_span.start(),
        bag_component.start + bc_span.end(),
        format!(
            "(window.__WUWA_PENDING_CHOICE__ === \"local\" ? Boolean({arg}?.asset_id) : (__lg.enabled.value ? __lg.canSelect({arg}) : {chk}({arg}))) && {bag_emit}(\"select-appearance\", {arg})",
            chk = &bag_click_m[1],
            arg = &bag_click_m[2],
        ),
    ));

    let rem_owned_re = Regex::new(&format!(
        r#"(const ([A-Za-z0-9_$]+) = [A-Za-z0-9_$]+\.bagList\.find\([^\n]+?\);\s*if \([A-Za-z0-9_$]+\) \{{\s*{}\("select-appearance", )([A-Za-z0-9_$]+)(\);)"#,
        regex::escape(&bag_emit)
    ))
    .unwrap();
    let rem_owned_m = rem_owned_re
        .captures_iter(bag_text)
        .find(|c| &c[2] == &c[3])
        .ok_or("MainPanelBagTab emote remove branch changed")?;
    let ro_span = rem_owned_m.get(0).unwrap();
    panel_edits.push(Edit::new(
        bag_component.start + ro_span.start(),
        bag_component.start + ro_span.end(),
        format!(
            "{}{{...{}, __wuwaRemove: !0}}{}",
            &rem_owned_m[1], &rem_owned_m[3], &rem_owned_m[4]
        ),
    ));
    let rem_fallback_re = Regex::new(&format!(
        r#"([A-Za-z0-9_$]+ && {}\("select-appearance", \{{\s*)(asset_code: "",)"#,
        regex::escape(&bag_emit)
    ))
    .unwrap();
    let rem_fb_m = rem_fallback_re
        .captures(bag_text)
        .ok_or("MainPanelBagTab emote remove fallback changed")?;
    let rf_span = rem_fb_m.get(0).unwrap();
    panel_edits.push(Edit::new(
        bag_component.start + rf_span.start(),
        bag_component.start + rf_span.end(),
        format!("{}__wuwaRemove: !0,\n          {}", &rem_fb_m[1], &rem_fb_m[2]),
    ));

    let bag_list_re = Regex::new(
        r"return ([A-Za-z0-9_$]+) \? ([A-Za-z0-9_$]+)\.bagList\.filter\(([A-Za-z0-9_$]+) => ([A-Za-z0-9_$]+)\.asset_type === ([A-Za-z0-9_$]+)\) : ([A-Za-z0-9_$]+)\.bagList",
    )
    .unwrap();
    let bl_m = bag_list_re
        .captures_iter(bag_text)
        .find(|c| &c[1] == &c[5] && &c[2] == &c[6] && &c[3] == &c[4])
        .ok_or("MainPanelBagTab bagList computed changed")?;
    let bl_span = bl_m.get(0).unwrap();
    panel_edits.push(Edit::new(
        bag_component.start + bl_span.start(),
        bag_component.start + bl_span.end(),
        format!(
            "const __mb=__lg.mergeBag?__lg.mergeBag({src}.bagList):{src}.bagList; return {tab} ? __mb.filter({it} => {it}.asset_type === {tab}) : __mb",
            src = &bl_m[2],
            tab = &bl_m[1],
            it = &bl_m[3],
        ),
    ));

    let bag_return = bag_component
        .return_start
        .ok_or("MainPanelBagTab 缺少 returnStart")?;
    panel_edits.push(Edit::new(
        bag_return,
        bag_return,
        format!(
            "let __disposeBagEquip=null;\n\
             {on_mounted_p}(()=>{{__disposeBagEquip=__lg.mountEquipmentBar?.(document.body,{{page:'bag',floating:true,emitSelect:x=>{bag_emit}('select-appearance',x)}})||null;}});\n\
             {on_unmounted_p}(()=>{{__disposeBagEquip?.();__disposeBagEquip=null;}});\n"
        ),
    ));

    let bag_guard = format!("if (!{global_store_p}.isLoggedIn)");
    if bag_text.match_indices(&bag_guard).count() != 2 {
        return Err("preset login guard layout changed".into());
    }
    for (idx, _) in bag_text.match_indices(&bag_guard) {
        panel_edits.push(Edit::new(
            bag_component.start + idx,
            bag_component.start + idx + bag_guard.len(),
            format!("if (!{global_store_p}.isLoggedIn && !__lg.enabled.value)"),
        ));
    }
    let delete_guard_re = Regex::new(&format!(
        r"if \([A-Za-z0-9_$]+\.stopPropagation\(\), !{}\.isLoggedIn\)",
        regex::escape(global_store_p)
    ))
    .unwrap();
    let del_m = delete_guard_re
        .find(bag_text)
        .ok_or("preset delete login guard changed")?;
    let del_str = del_m.as_str();
    panel_edits.push(Edit::new(
        bag_component.start + del_m.start(),
        bag_component.start + del_m.end(),
        format!(
            "{} && !__lg.enabled.value)",
            &del_str[..del_str.len() - 1]
        ),
    ));

    let mut panel = apply_edits(&panel_orig, panel_edits)?;
    panel = rec.replace(
        &panel,
        "= [\"common\", \"uncommon\", \"rare\", \"epic\"];",
        "= [\"mod\", \"common\", \"uncommon\", \"rare\", \"epic\"];",
    )?;
    let bs_re = Regex::new(
        r"const ([A-Za-z0-9_$]+) = [A-Za-z0-9_$]+\.trim\(\)\.toLowerCase\(\);\s*return [A-Za-z0-9_$]+\(`select_rarity_\$\{([A-Za-z0-9_$]+)\}`",
    )
    .unwrap();
    let bs_m = bs_re
        .captures_iter(&panel)
        .find(|c| &c[1] == &c[2])
        .ok_or("BagAtlasFilters rarity label helper changed")?;
    let bs_whole = bs_m.get(0).unwrap().as_str().to_owned();
    let bs_var = bs_m[1].to_owned();
    let bs_new = bs_whole.replacen(
        "return ",
        &format!("if ({bs_var} === \"mod\") return \"MOD\";\n  return "),
        1,
    );
    panel = rec.replace(&panel, &bs_whole, &bs_new)?;
    let panel_accessory_alias = require_import(&pi, "getUserAccessoryOffset")?;
    if let Ok(patched) = patch_accessory_transform(&mut rec, &panel, panel_accessory_alias) {
        panel = patched;
    }
    let update_marker = "\"setting-btn btn-check-update\"";
    if panel.match_indices(update_marker).count() != 1 {
        return Err("official update button contract changed".into());
    }
    let marker_at = panel.find(update_marker).unwrap();
    let call_at = panel[marker_at..]
        .find("onClick:")
        .map(|o| marker_at + o)
        .ok_or("official update button onClick missing")?;
    let mod_call = format!("{with_modifiers}(");
    let handler_rel = panel[call_at..]
        .find(&mod_call)
        .map(|o| call_at + o + mod_call.len())
        .ok_or("official update button withModifiers( missing")?;
    let handler_end = panel[handler_rel..]
        .find(',')
        .map(|o| handler_rel + o)
        .ok_or("official update button handler delimiter missing")?;
    let raw_handler = &panel[handler_rel..handler_end];
    let handler = raw_handler.trim();
    if handler.is_empty()
        || !handler
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
    {
        return Err(format!("official update handler invalid: {raw_handler}"));
    }
    let trimmed_start = handler_rel + raw_handler.find(handler).unwrap();
    panel.replace_range(
        trimmed_start..trimmed_start + handler.len(),
        "__openCommunityUpdate",
    );

    let unfav = require_import(&pi, "unfavoriteAsset")?;
    let fav = require_import(&pi, "favoriteAsset")?;
    let fav_re = Regex::new(&format!(
        r"([A-Za-z0-9_$]+)\.value \? await ([A-Za-z0-9_$]+)\(await {}\(([A-Za-z0-9_$]+)\.item\.asset_id\)\) : await ([A-Za-z0-9_$]+)\(await {}\(([A-Za-z0-9_$]+)\.item\.asset_id\)\)",
        regex::escape(unfav),
        regex::escape(fav)
    ))
    .unwrap();
    let fav_caps = fav_re
        .captures_iter(&panel)
        .find(|c| &c[2] == &c[4] && &c[3] == &c[5])
        .ok_or("Favorite toggle layout changed")?;
    let fav_pos = fav_caps.get(0).unwrap().start();
    let fav_flag = fav_caps[1].to_owned();
    let fav_item = fav_caps[3].to_owned();
    panel.insert_str(
        fav_pos,
        &format!("(__lg.isModAsset?.({fav_item}.item) || (__lg.enabled.value && {fav_item}.item?.owned === false)) ? __lg.favorite({fav_item}.item.asset_id,!{fav_flag}.value) : "),
    );

    // Phase 2: Desktop assistant + platform hooks (assistant_core + WindowsHooks)
    let mut facade_files = vec!["assistant_dashboard.js", "desktop_assistant.js"];
    if platform == Platform::Windows {
        facade_files.push("desktop_context_menu.js");
    }
    let mut factory_parts = Vec::with_capacity(facade_files.len());
    for fname in facade_files {
        factory_parts.push(render_frontend_module(repo_root, fname, platform)?);
    }
    let factory = factory_parts.join("\n");

    let mut handlers = Vec::with_capacity(2);
    for event in ["key-press", "key-release"] {
        let listen_sym = require_import(&mi, "listen")?;
        let re = Regex::new(&format!(
            r#"{}\("{event}", (\w+) => \{{\s*([A-Za-z0-9_$]+)\(([A-Za-z0-9_$]+)\.payload\)"#,
            regex::escape(listen_sym)
        ))
        .unwrap();
        let caps = re
            .captures_iter(&main_orig)
            .find(|c| &c[1] == &c[3])
            .ok_or_else(|| format!("缺少键盘事件监听：{event}"))?;
        handlers.push(caps[2].to_owned());
    }
    let press_fn = named(&m_raw.functions, &handlers[0])?;
    let press_text = &main_orig[press_fn.start..press_fn.end];
    let first_guard_caps = Regex::new(r"if \(([^\n]+)\) return;")
        .unwrap()
        .captures(press_text)
        .ok_or("按键处理缺少守卫条件")?;
    let press_param = param_of(&handlers[0])?.unwrap_or_else(|| "e".into());
    let release_param = param_of(&handlers[1])?.unwrap_or_else(|| "e".into());
    let held_guard = Regex::new(&format!(
        r"[A-Za-z0-9_$]+\.has\({}\)",
        regex::escape(&press_param)
    ))
    .unwrap()
    .find_iter(&first_guard_caps[1])
    .last()
    .ok_or("按键守卫缺少 held_guard")?
    .as_str()
    .to_owned();
    main = rec.replace(
        &main,
        &format!("function {}({press_param}) {{", handlers[0]),
        &format!("function {}({press_param},__assistantVirtual=false) {{", handlers[0]),
    )?;
    main = rec.replace(
        &main,
        &first_guard_caps[0],
        &format!(
            "if (__assistantVirtual ? {held_guard} : ({})) return;\n        if (!__assistantVirtual) __lg.assistant?.onGlobalKey?.({press_param}, true);",
            &first_guard_caps[1]
        ),
    )?;
    main = rec.replace(
        &main,
        &format!("function {}({release_param}) {{", handlers[1]),
        &format!(
            "function {}({release_param}) {{\n        __lg.assistant?.onGlobalKey?.({release_param}, false);",
            handlers[1]
        ),
    )?;

    let chest_fn = function_for(&m_raw.functions, &main_orig, "openChest failed")?;
    let chest_text = &main_orig[chest_fn.start..chest_fn.end];
    let cost = Regex::new(&format!(r"{}\.totalClickCount < ([A-Za-z0-9_$]+)", regex::escape(store)))
        .unwrap()
        .captures(chest_text)
        .ok_or("chest cost anchor missing")?[1]
        .to_owned();
    let anim_caps = Regex::new(r"const (\w+) = ([A-Za-z0-9_$]+)\(e\);")
        .unwrap()
        .captures(chest_text)
        .ok_or("chest animation anchor missing")?;
    let anim_var = anim_caps[1].to_owned();
    let animation = anim_caps[2].to_owned();

    let ready_re = Regex::new(&format!(
        r"([A-Za-z0-9_$]+) = {}\(\(\) => ([A-Za-z0-9_$]+)\.value \|\| ![A-Za-z0-9_$]+\.isLoggedIn \? !0 : ([A-Za-z0-9_$]+)\.value <= 0\)",
        regex::escape(computed_sym)
    ))
    .unwrap();
    let ready_pairs: Vec<(String, String)> = ready_re
        .captures_iter(&main_orig)
        .map(|c| (c[1].to_owned(), c[3].to_owned()))
        .collect();
    if ready_pairs.len() != 2 {
        return Err(format!(
            "chest readiness contract (expected two ready computeds, got {})",
            ready_pairs.len()
        ));
    }
    let (left_ready, left_rem) = &ready_pairs[0];
    let (right_ready, right_rem) = &ready_pairs[1];
    let busy_caps = Regex::new(r"if \(([A-Za-z0-9_$]+)\.value \|\| ([A-Za-z0-9_$]+)\.value\) return;")
        .unwrap()
        .captures(chest_text)
        .ok_or("chest busy guard contract missing")?;
    let (busy_open, blocked_chest) = (&busy_caps[1], &busy_caps[2]);

    shared = rec.replace(
        &shared,
        "c === 10002 || c === 10003 || r === 401 || r === 403",
        "c === 10002 || c === 10003 || r === 401",
    )?;
    shared = rec.replace(
        &shared,
        "function isAppUpdaterEnabled() {",
        "function isAppUpdaterEnabled() {\n return false;",
    )?;
    shared = rec.replace(
        &shared,
        "function isForceUpdateLocked() {",
        "function isForceUpdateLocked() {\n return window.__WUWA_LOCAL_ATLAS__?.assistant?.updateRequired.value === true;",
    )?;
    shared = rec.replace(
        &shared,
        "function schedulePreloadAppUiImages() {",
        "function schedulePreloadAppUiImages() {\n return;",
    )?;
    shared = rec.replace(
        &shared,
        "gameMode: t(\"setting_game_mode\"),",
        "gameMode: t(\"setting_game_mode\"),\n    switchDisplay: t(\"setting_screen_btn\"),",
    )?;

    let report_sticker = require_import(&mi, "reportSendSticker")?;
    let emote_fn = function_for(&m_raw.functions, &main_orig, &format!("{report_sticker}({{"))?;
    let emote_text = &main_orig[emote_fn.start..emote_fn.end];
    if !emote_text.contains("e.src") || !emote_text.contains(".push({") {
        return Err("manual emote rendering contract changed".into());
    }
    let emote_items = Regex::new(&format!(
        r#""emote-items": ([A-Za-z0-9_$]+)\.value,\s*onSelect: {}"#,
        regex::escape(&emote_fn.name)
    ))
    .unwrap()
    .captures(&main_orig)
    .ok_or("emote selection binding changed")?[1]
        .to_owned();
    let emote_url = Regex::new(&format!(
        r#""asset-url": ([A-Za-z0-9_$]+),\s*"emote-items": {}"#,
        regex::escape(&emote_items)
    ))
    .unwrap()
    .captures(&main_orig)
    .ok_or("emote asset url binding changed")?[1]
        .to_owned();
    let sync_call_caps = Regex::new(&format!(
        r"{}\.isLoggedIn && ([A-Za-z0-9_$]+)\(e\.id\);",
        regex::escape(store)
    ))
    .unwrap()
    .captures(emote_text)
    .ok_or("original room emote send contract changed")?;
    main = rec.replace(
        &main,
        &sync_call_caps[0],
        &format!(
            "{store}.isLoggedIn && (!__lg.enabled.value || __lg.officialOwned?.(e.id, 'emote')) && {}(e.id);",
            &sync_call_caps[1]
        ),
    )?;

    let mut facade_locals = String::from(
        "\nif(localAtlasController)localAtlasController.assistant=createDesktopAssistant({ref,window,atlas:localAtlasController,displayName:resolveAssetDisplayName});\nwindow.__WUWA_OPEN_PANEL_TAB__=tab=>invoke(\"open_panel_window\",{payload:{tab}}).then(()=>true).catch(()=>false);",
    );
    if platform == Platform::Windows {
        facade_locals.push_str(
            "\nif(localAtlasController)localAtlasController.contextMenu=createDesktopContextMenu({invoke,request:(path,body)=>localAtlasController.request(path,body),settings:loadDesktopCatSettings,visible:()=>petDockVisibleForMenu,labels:async()=>{try{return await getNativeMenuLabels(formatShortcut((await getAppShortcuts()).togglePet));}catch(error){return {};}},report:details=>localAtlasController.request('/diagnostic',details)});",
        );
    }
    shared = rec.replace(
        &shared,
        "window.__WUWA_LOCAL_ATLAS__ = localAtlasController;",
        &format!("{factory}\nwindow.__WUWA_LOCAL_ATLAS__ = localAtlasController;{facade_locals}"),
    )?;
    shared = rec.replace(
        &shared,
        "localAtlasController.ready.then(async () => { localStartOnlineServices(); await bootstrap(); localAtlasController.startOverlay(); })",
        "Promise.all([localAtlasController.ready,localAtlasController.assistant.ready]).then(async () => { localStartOnlineServices(); await bootstrap(); localAtlasController.startOverlay(); })",
    )?;

    main = rec.replace(
        &main,
        &format!("const {anim_var} = {animation}(e);"),
        &format!(
            "const __assistantAttempt=__lg.assistant.beginChest(e);\n        const {anim_var} = {animation}(e);"
        ),
    )?;
    let report_reward_succ = require_import(&mi, "reportRewardClaimSucc")?;
    let reward_re = Regex::new(&format!(
        r"(?m)^([ \t]*){}\(\{{\s*\n\s*itemId:\s*([A-Za-z0-9_$]+)\.data\.asset_id,",
        regex::escape(report_reward_succ)
    ))
    .unwrap();
    let reward_caps = reward_re
        .captures(&main)
        .ok_or("chest reward success anchor missing")?;
    let reward_whole = reward_caps[0].to_owned();
    let reward_indent = &reward_caps[1];
    let reward_var = &reward_caps[2];
    main = rec.replace(
        &main,
        &reward_whole,
        &format!(
            "{reward_indent}__lg.assistant.chestSucceeded(__assistantAttempt,{reward_var}.data);\n{reward_whole}"
        ),
    )?;

    let room_field = req_field("currentRoomId")?;
    let room_members_field = req_field("roomMembers")?;
    let members = format!(
        "{}.value",
        &Regex::new(r"^([A-Za-z0-9_$]+)\.value")
            .unwrap()
            .captures(room_members_field)
            .ok_or("roomMembers 格式异常")?[1]
    );
    let input_room = req_field("roomIdInput")?;
    let join_fn = m_raw
        .cases
        .get("join-room")
        .ok_or("缺少 case join-room")?;
    let with_mod_m = require_import(&mi, "withModifiers")?;
    let open_settings = Regex::new(&format!(
        r#"class: "settings-btn"[\s\S]{{0,300}}?onClick: {}\(([A-Za-z0-9_$]+),"#,
        regex::escape(with_mod_m)
    ))
    .unwrap()
    .captures(&main_orig)
    .ok_or("settings-btn onClick 锚点丢失")?[1]
        .to_owned();
    let open_tab = function_for(&m_raw.functions, &main_orig, "[panel] open tab failed")?
        .name
        .clone();
    let quit_fn = Regex::new(r#""tray-quit", \(\) => \{\s*([A-Za-z0-9_$]+)\(\)"#)
        .unwrap()
        .captures(&main_orig)
        .ok_or("tray-quit 锚点丢失")?[1]
        .to_owned();
    let persist_imp = require_import(&mi, "listenPersistClicksBeforeExit")?;
    let persist_original = Regex::new(&format!(
        r"(?s){}\(async \(\) => \{{\s*([\s\S]+?)\n\s*\}}\)",
        regex::escape(persist_imp)
    ))
    .unwrap()
    .captures(&main_orig)
    .ok_or("listenPersistClicksBeforeExit 锚点丢失")?[1]
        .to_owned();
    main = rec.replace(
        &main,
        &format!("async function {quit_fn}() {{"),
        &format!("async function {quit_fn}() {{\n try{{await __lg.assistant.flush();}}catch{{}}"),
    )?;

    let tab_pos = main
        .find(&format!("function {open_tab}("))
        .ok_or("open_tab definition missing in patched main")?;
    let search_start = find_char_boundary_backward(&main, tab_pos.saturating_sub(6));
    let search_end = {
        let mut e = (tab_pos + 600).min(main.len());
        while e < main.len() && !main.is_char_boundary(e) {
            e += 1;
        }
        e
    };
    let tab_guard_re = Regex::new(&format!(
        r"(?:async )?function {}\(([A-Za-z0-9_$]+)\)\s*\{{\s*if \(([A-Za-z0-9_$]+)\(\)\) \{{\s*{}\(",
        regex::escape(&open_tab),
        regex::escape(show_toast_sym)
    ))
    .unwrap();
    let tab_caps = tab_guard_re
        .captures(&main[search_start..search_end])
        .ok_or("panel update-lock guard changed")?;
    let old_guard = tab_caps[0].to_owned();
    let tab_param = &tab_caps[1];
    let lock_fn = &tab_caps[2];
    let new_guard = old_guard.replacen(
        &format!("if ({lock_fn}()) {{"),
        &format!("if ({tab_param} !== \"console\" && {lock_fn}()) {{"),
        1,
    );
    main = rec.replace(&main, &old_guard, &new_guard)?;

    let mut code_prefix = String::new();
    let mut blocked_expr = "__lg.assistant.updateRequired.value".to_owned();
    let on_mounted_m = require_import(&mi, "onMounted")?;
    if platform == Platform::Windows {
        let (patched_main, ctx_patch) =
            patch_windows_context_menu(&main, &main_orig, &m_raw, &mi, &open_tab)?;
        main = patched_main;
        code_prefix = format!(
            "\n{on_mounted_m}(()=>{{window.setTimeout(()=>__lg?.contextMenu?.initialize({}).catch(error=>console.warn('[context-menu] initialize',error)),1500);}});",
            ctx_patch.context_actions
        );
        blocked_expr = "__lg.assistant.updateRequired.value||Boolean(__lg.contextMenu?.isOpen())".into();
    }

    let invoke_m = require_import(&mi, "invoke$1")?;
    let listen_m = require_import(&mi, "listen")?;
    let on_unmounted_m = require_import(&mi, "onUnmounted")?;
    let code = format!(
        "{code_prefix}\n\
let __unlistenSwitchDisplay=null;\n\
{on_mounted_m}(async()=>{{__unlistenSwitchDisplay=await {listen_m}('tray-switch-display',async()=>{{if(__lg.assistant?.updateRequired?.value)return;try{{const ok=await window.__WUWA_SWITCH_DISPLAY__?.();if(ok===false){show_toast_sym}('未检测到其他可切换的显示器');}}catch(error){{{show_toast_sym}(error?.message||'切换屏幕失败');}}}});}});\n\
{on_unmounted_m}(()=>{{__unlistenSwitchDisplay?.();}});\n\
{on_mounted_m}(()=>{{window.setTimeout(()=>__lg.assistant.startMain({{\n\
 total:()=>{store}.cumulativeClickCount,balance:()=>{store}.totalClickCount,\n\
 account:()=>String({store}.userInfo?.cuid||{store}.userInfo?.user_id||{store}.userInfo?.uid||'guest'),\n\
 loggedIn:()=>{store}.isLoggedIn,blocked:()=>{blocked_expr},\n\
 press:key=>{press_h}(key,true),release:{release_h},chestCost:{cost},\n\
 chests:()=>({{left:{{remaining:Math.max(0,{left_rem}.value),ready:{left_ready}.value}},right:{{remaining:Math.max(0,{right_rem}.value),ready:{right_ready}.value}},cooldown:1800000,busy:{busy_open}.value||{blocked_chest}.value}}),\n\
 emotes:()=>({emote_items}.value||[]).map(item=>({{id:item.id,name:item.name,src:item.src||(item.name?((window.__WUWA_MOD_ASSET_PATH__&&window.__WUWA_MOD_ASSET_PATH__('emote/'+item.name))||{emote_url}('emote/'+item.name)):'')}})),\n\
 sendEmote:item=>{emote_send}(item),\n\
 toast:{show_toast_sym},\n\
 openSettings:{open_settings},\n\
 openView:async view=>{{\n\
   if(__lg.assistant.updateRequired.value)return;\n\
   await {open_tab}(view==='atlas'?'atlas':(['settings','setting'].includes(view)?'setting':'console'));\n\
   const send=()=>{{{invoke_m}('plugin:event|emit',{{event:'local-assistant-view',payload:view}}).catch(()=>{{}});}};\n\
   send();window.setTimeout(send,800);window.setTimeout(send,2200);\n\
 }},\n\
 quit:{quit_fn},\n\
 persistOriginal:async()=>{{{persist_original}}},\n\
 refreshBag:async()=>{{await {bag_fn}();}},\n\
 listenClicks:callback=>{watch_sym}(()=>{store}.localClickCount,(next,prev)=>{{if(Number(next)===Number(prev)+1)callback(1);}},{{flush:'sync'}}),\n\
 listenRoom:callback=>{watch_sym}([()=>{room_field},()=>{members}.map(x=>[x.id,x.nickname,x.isOwner]).join('|')],()=>{{
   const owner={members}.find(x=>x.isOwner);if({room_field})callback(String({room_field}),owner?.nickname||'').catch(()=>{{}});
 }},{{immediate:true}}),\n\
 listenJoin:callback=>{listen_m}('local-assistant-join',event=>callback(String(event.payload))),\n\
 join:async code=>{{{input_room}=code;await {join_fn}();}}\n\
}}).catch(error=>console.error('[assistant]',error)),3000);}});\n",
        press_h = handlers[0],
        release_h = handlers[1],
        emote_send = emote_fn.name,
        bag_fn = bag_loader.name,
    );
    main = rec.replace(
        &main,
        "let __localSaved=null,__skipLocalSelectChoice=false,__skipLocalReorder=false,__officialApplyDepth=0;",
        &format!("{code}\nlet __localSaved=null,__skipLocalSelectChoice=false,__skipLocalReorder=false,__officialApplyDepth=0;"),
    )?;

    let monitor_name = fs::read_dir(&readable_assets)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|name| name.starts_with("switchDisplayMonitor-") && name.ends_with(".js"))
        .ok_or("缺少 switchDisplayMonitor-*.js 模块")?;
    let mut monitor = read_normalized_js(&readable_assets.join(&monitor_name))?;
    let rn_re = Regex::new(
        r"return \[\.\.\.([A-Za-z0-9_$]+)\]\.sort\(\(([A-Za-z0-9_$]+), ([A-Za-z0-9_$]+)\) => \{\s*if \(([A-Za-z0-9_$]+)\.owned !== ([A-Za-z0-9_$]+)\.owned\) return ([A-Za-z0-9_$]+)\.owned \? -1 : 1;",
    )
    .unwrap();
    let rn_caps = rn_re
        .captures_iter(&monitor)
        .find(|c| &c[2] == &c[4] && &c[2] == &c[6] && &c[3] == &c[5])
        .ok_or("atlas flat sort rn() anchor changed")?;
    let rn_whole = rn_caps[0].to_owned();
    let rn_a = rn_caps[2].to_owned();
    let rn_b = rn_caps[3].to_owned();
    let rn_new = rn_whole.replacen(
        &format!("if ({rn_a}.owned !== {rn_b}.owned)"),
        &format!(
            "const __tm=({rn_a}.rarity||\"\").toLowerCase()===\"mod\",__am=({rn_b}.rarity||\"\").toLowerCase()===\"mod\";if(__tm!==__am)return __tm?-1:1;\n    if ({rn_a}.owned !== {rn_b}.owned)"
        ),
        1,
    );
    monitor = rec.replace(&monitor, &rn_whole, &rn_new)?;
    let tab_guard =
        "return e === \"bag\" || e === \"atlas\" || e === \"exchange\" || e === \"room\" || e === \"setting\"";
    monitor = rec.replace(&monitor, tab_guard, &format!("{tab_guard} || e === \"console\""))?;
    monitor = patch_monitor_switch_display(&mut rec, &monitor)?;
    if platform == Platform::Windows {
        monitor = patch_windows_monitor_autostart(&monitor);
    }
    fs::write(frontend_dir.join("assets").join(&monitor_name), monitor.as_bytes())
        .map_err(|e| e.to_string())?;

    let assistant_row = format!(
        "{h}('div',{{class:'setting-item','data-desktop-assistant':'true'}},[{h}('div',{{class:'setting-label'}},[{h}('div',{{class:'setting-title'}},'桌宠助手'),{h}('div',{{class:'setting-sub'}},'自动点击与开箱、每日统计、Mod 与挂饰及版本更新')]),{h}('button',{{class:'setting-btn',type:'button',onClick:()=>__lg.assistant.openPanel()}},'打开控制台')]),"
    );
    let gallery_anchor =
        format!("{h}('div',{{class:'setting-item','data-local-gallery-setting':'true'}},[");
    panel = rec.replace(
        &panel,
        &gallery_anchor,
        &format!("{assistant_row}\n{gallery_anchor}"),
    )?;

    let pair_re = Regex::new(r"await [A-Za-z0-9_$]+\(\), await [A-Za-z0-9_$]+\(\)").unwrap();
    let pair_calls: HashSet<String> = pair_re
        .find_iter(&panel)
        .map(|m| m.as_str().to_owned())
        .collect();
    if !pair_calls
        .iter()
        .any(|call| panel.match_indices(call.as_str()).count() == 2)
        && panel.match_indices("await ds(), await xt()").count() != 2
    {
        return Err("panel close lifecycle changed".into());
    }
    let listen_p = require_import(&pi, "listen")?;
    panel = rec.replace(
        &panel,
        "let __panelDisposed=false;",
        &format!(
            "let __assistantViewUnlisten=null;\n\
{on_mounted_p}(async()=>{{__assistantViewUnlisten=await {listen_p}('local-assistant-view',event=>{{\n\
 const view=event.payload;\n\
 if(['overview','rewards','daily','automation','records','rooms','mods','system','update'].includes(view))__lg.assistant.openPanel(view).catch(error=>console.error('[assistant-view]',error));\n\
 else if(['atlas','settings','setting'].includes(view))__lg.assistant.closePanel();\n\
}});}});\n\
{on_unmounted_p}(()=>{{__assistantViewUnlisten?.();}});\n\
let __panelDisposed=false;"
        ),
    )?;

    for (name, text) in [
        (&shared_name, &shared),
        (&main_name, &main),
        (&panel_name, &panel),
    ] {
        fs::write(frontend_dir.join("assets").join(name), text.as_bytes())
            .map_err(|e| e.to_string())?;
    }

    let mut extra_patched_assets = Vec::new();
    for entry in fs::read_dir(base_dir.join("frontend/assets")).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let fname = entry.file_name().to_string_lossy().into_owned();
        if !fname.starts_with("switchDisplayMonitor-") || !fname.ends_with(".js") {
            continue;
        }
        let orig_bytes = fs::read(entry.path()).map_err(|e| e.to_string())?;
        let new_bytes = fs::read(frontend_dir.join("assets").join(&fname)).map_err(|e| e.to_string())?;
        if orig_bytes != new_bytes {
            extra_patched_assets.push(fname);
        }
    }

    let manifest = json!({
        "shared": shared_name,
        "main": main_name,
        "panel": panel_name,
        "settingsSymbol": settings.variable,
        "atlasSymbol": atlas_component.variable,
        "base": base_dir.to_string_lossy(),
        "version": official_version,
    });
    fs::write(
        build_root.join("build_manifest_v2.json"),
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;

    Ok(IntegrationResult {
        manifest,
        patch_log: rec.entries,
        shared_name,
        main_name,
        panel_name,
        extra_patched_assets,
        base_dir,
        frontend_dir,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_asserts_anchor_count() {
        let mut rec = PatchRecorder::default();
        assert_eq!(rec.replace_n("a b a", "a", "x", 2).unwrap(), "x b x");
        assert!(rec.replace("a b a", "a", "x").is_err());
        assert!(rec.replace("a b", "zzz", "x").is_err());
    }

    #[test]
    fn apply_edits_keeps_earlier_offsets_stable() {
        let edits = vec![Edit::new(8, 10, "AB"), Edit::new(0, 1, "zero")];
        assert_eq!(apply_edits("0123456789", edits).unwrap(), "zero1234567AB");
        assert!(apply_edits("abc", vec![Edit::new(0, 99, "x")]).is_err());
    }

    #[test]
    fn char_to_byte_map_handles_multibyte_chinese_and_astral_chars() {
        let text = "const 提示='中文😀'; function target() { return 1; }";
        let map = char_to_byte_map(text);
        let char_pos = text.chars().take_while(|&c| c != '{').count();
        let byte_pos = map_offset(&map, char_pos, "body").unwrap();
        assert_eq!(&text[byte_pos..byte_pos + 1], "{");
        let meta = vec![FunctionMeta {
            name: "target".into(),
            start: map[text.chars().count() - 28],
            end: text.len(),
            body: byte_pos,
            body_end: text.len(),
        }];
        assert_eq!(function_for(&meta, text, "return 1").unwrap().name, "target");
    }

    #[test]
    fn function_for_picks_innermost_and_named_requires_unique() {
        let functions = vec![
            FunctionMeta {
                name: "outer".into(),
                start: 0,
                end: 20,
                body: 5,
                body_end: 20,
            },
            FunctionMeta {
                name: "inner".into(),
                start: 5,
                end: 15,
                body: 10,
                body_end: 15,
            },
        ];
        let text = &"x".repeat(30);
        assert_eq!(function_for(&functions, text, "x").unwrap().name, "inner");
        assert!(function_for(&functions[..1], "abc", "zzz").is_err());
        assert_eq!(named(&functions, "outer").unwrap().end, 20);
        let dup = vec![functions[0].clone(), functions[0].clone()];
        assert!(named(&dup, "outer").is_err());
    }

    #[test]
    fn shared_patches_match_contract() {
        let mut rec = PatchRecorder::default();
        let url_out =
            patch_mod_asset_url(&mut rec, "function buildAssetUrl(e, t) {\n  return 1\n}")
                .unwrap();
        assert!(url_out.contains("__WUWA_MOD_ASSET_PATH__(t)"));
        assert!(url_out.contains("if (modAssetPath) return modAssetPath;"));
        assert!(!url_out.contains("withAssetUrlCacheBust"));
        assert!(patch_mod_asset_url(&mut rec, "function buildAssetUrl(e) {\n}").is_err());

        let name_out = patch_mod_display_name(&mut rec, MOD_NAME_FALLBACK_OLD).unwrap();
        assert!(name_out.contains("__WUWA_MOD_NAMES__"));
        assert!(!name_out.contains("return s === \"character_skin\""));

        let online_out = patch_online_services(
            &mut rec,
            "loaderModule.init(kurodataConfig);\nwindow.kurodata = loaderModule;\ninitTrackCommonProps();\ninitFrontendAlertReporting();",
        )
        .unwrap();
        assert!(online_out.contains("function localStartOnlineServices() {"));
        assert!(online_out.contains("if (localOnlineServicesStarted) return;"));

        let inv_out = patch_invoke_guard(
            &mut rec,
            "async function invoke$1(e, t = {}, n) {\n}\n",
            false,
        )
        .unwrap();
        assert!(inv_out.contains("checkInvoke(e, t)"));
        assert!(patch_invoke_guard(&mut rec, "no wrapper here", true).is_err());
        assert_eq!(
            patch_invoke_guard(&mut rec, "no wrapper here", false).unwrap(),
            "no wrapper here"
        );

        let guard_src = "async function ensureAppearanceAssetCatalog(e) {\n}\nfunction applyCatalog(e, t, n) {\n}\nfunction atlas() {\n}\nthis.instance.interceptors.request.use(t => {\n";
        let guard_out = patch_atlas_guards(&mut rec, guard_src, "\n PLATFORM_EXTRA").unwrap();
        assert!(guard_out.contains("api/common/activity-config"));
        assert!(guard_out.contains("PLATFORM_EXTRA"));
        assert!(guard_out.contains("offlineError()"));
        assert_eq!(
            guard_out
                .match_indices("this.instance.interceptors.request.use")
                .count(),
            1
        );

        let taskbar_out = patch_taskbar_default(
            &mut rec,
            "const defaults = { showTaskbarIcon: !0 };\nfunction clampPetScale(e) {}",
        )
        .unwrap();
        assert!(taskbar_out.contains("showTaskbarIcon: !1"));
        assert!(taskbar_out.contains("wuwatappo-settings-community-taskbar-v1"));

        for invoke_fn in ["invoke", "invoke$1"] {
            let ver_src = format!(
                "try {{ cachedAppVersion = await getVersion() }} catch {{}}\nasync function getVersion() {{ return {invoke_fn}(\"plugin:app|version\") }}\n"
            );
            let ver_out = patch_display_version(&ver_src, "1.3.2-beta.2", "1.0.4").unwrap();
            assert!(ver_out.contains("cachedAppVersion = \"1.0.4\""));
            assert!(ver_out.contains("c + \" · 基底 \" + b"));
            assert!(ver_out.contains("\"/community/version\""));
            assert!(!ver_out.contains("plugin:app|version"));
        }
    }

    #[test]
    fn render_frontend_module_substitutes_platform_placeholders() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let mac_da = render_frontend_module(repo, "desktop_assistant.js", Platform::Mac).unwrap();
        let win_da =
            render_frontend_module(repo, "desktop_assistant.js", Platform::Windows).unwrap();
        assert!(mac_da.contains("macOS input queue"));
        assert!(win_da.contains("Windows input queue"));
        assert!(!mac_da.contains("__WUWA_ASSISTANT_DEFAULTS__"));

        let mac_dash =
            render_frontend_module(repo, "assistant_dashboard.js", Platform::Mac).unwrap();
        let win_dash =
            render_frontend_module(repo, "assistant_dashboard.js", Platform::Windows).unwrap();
        assert!(mac_dash.contains("WUWA TAPPO FULLATLAS / MAC"));
        assert!(win_dash.contains("WUWA TAPPO / 后台助手"));
    }

    #[test]
    fn accessory_transform_patch_wraps_the_official_computed() {
        let source = "const Be = V(gi),\n        sn = pe(() => {\n          const {\n            x: e,\n            y: t\n          } = Du(A.userInfo);\n          return {\n            transform: `translate(${e}px, ${t}px)`\n          }\n        }),\n        next = 1;";
        let mut rec = PatchRecorder::default();
        let out = patch_accessory_transform(&mut rec, source, "Du").unwrap();
        assert!(out.contains("accessoryTransform?.(A.userInfo)"), "{out}");
        assert!(out.contains("rotate(${__t.rotate}deg)"), "{out}");
        assert!(out.contains("sn=pe(()=>"), "{out}");
        assert!(!out.contains("NAME=pe(()=>"), "{out}");
    }

    #[test]
    fn emotion_menu_raf_patch_injects_idle_and_fps_guards() {
        let source = r#"function De(X) {
  const U = Math.min((X - C) / 1e3, .028);
  C = X;
  for (const [Q, z] of M) z.style.transform = `translate(${Q}px, ${z}px)`;
  L = requestAnimationFrame(De)
}
return al(() => {
  C = performance.now(), L = requestAnimationFrame(De), window.addEventListener("resize", f)
}), Di(() => {
  cancelAnimationFrame(L), window.removeEventListener("resize", f), r(); console.log("cleanup")
}), (A, B) => ce.value ? A("open") : null;
"#;
        let mut rec = PatchRecorder::default();
        let out = patch_emotion_menu_raf(&mut rec, source, "watch").unwrap();
        assert!(out.contains("let __wuwaRafLast = 0;\nfunction De(X) {"), "{out}");
        assert!(out.contains("const __p = __lg.perf?.value || {};"), "{out}");
        assert!(
            out.contains("if (__p.idlePause !== false && !ce.value) { L = 0; return; }"),
            "{out}"
        );
        assert!(out.contains("const __cap = Number(__p.fpsCap) || 0;"), "{out}");
        assert!(
            out.contains("if (!(__cap > 0 && __wuwaRafLast > 0 && X - __wuwaRafLast < 1000 / __cap - 1)) { __wuwaRafLast = X;"),
            "{out}"
        );
        assert!(
            out.contains("const U = Math.min((X - C) / 1e3, .028); C = X; for (const [Q, z] of M) z.style.transform = `translate(${Q}px, ${z}px)`; }"),
            "{out}"
        );
        assert!(out.contains("\n L = requestAnimationFrame(De)\n}"), "{out}");
        assert!(!out.contains("requestAnimationFrame(De); return; }"), "{out}");
        assert_eq!(out.match_indices("requestAnimationFrame").count(), 2);
        assert!(
            out.contains("function __wuwaWakeRaf() { if (!L) { C = performance.now(); __wuwaRafLast = 0; L = requestAnimationFrame(De); } }"),
            "{out}"
        );
        assert!(
            out.contains("watch([ce, () => __lg.perf?.value?.idlePause], ([__open, __idle]) => { if (__open || __idle === false) __wuwaWakeRaf(); }, { flush: \"sync\" });"),
            "{out}"
        );
        assert!(
            out.contains("return al(() => { window.addEventListener(\"resize\", f); __wuwaWakeRaf(); }), Di(() => { cancelAnimationFrame(L); L = 0; window.removeEventListener(\"resize\", f); r(); console.log(\"cleanup\") }), (A, B) => ce.value ?"),
            "{out}"
        );

        let broken = source.replace("C = performance.now()", "D = performance.now()");
        assert!(patch_emotion_menu_raf(&mut rec, &broken, "watch").is_err());
    }

    #[test]
    fn room_layout_hooks_patch_visibility_restore_and_actions() {
        let source = r#"function vo(e) { return fe.value && e !== go.value || xo.value.has(e) }
function ep(t) { if (pt.has(t.id) || !ready.value) return; const u = zr.value?.querySelector(".cat-wrap"); if (!u) return; const o = u.getBoundingClientRect(), l = o.width / 800; if (!(l > 0) || !Number.isFinite(o.left) || !Number.isFinite(o.top)) return; const d = cl(o.left + t.left * l, o.top + t.top * l); t.left = d.left, t.top = d.top, pt.add(t.id), zi(t) }
async function ml() { if (await tick(), !!ready.value) { for (const [k, v] of members.value.entries()) il(v, k), ep(v); ta(), await tick(), ea() } }
function jn() { ml() }
function drag(x) { x && hit(x) && (bounce ? settle(x, { left: tx, top: oy }) : (x.left = pos.left, x.top = pos.top, settle(x, pos))); }
"#;
        let mut rec = PatchRecorder::default();
        let out = patch_room_layout_hooks(&mut rec, source, "onUnmounted").unwrap();
        assert!(
            out.contains("function vo(e) { return fe.value && e !== go.value || xo.value.has(e) || Boolean(__lg.isRoomMemberHidden?.(e)) }"),
            "{out}"
        );
        assert!(
            out.contains("const __others = members.value.filter(__m => __m.id !== t.id && pt.has(__m.id)).map(__m => ({ left: __m.left, top: __m.top }));"),
            "{out}"
        );
        assert!(
            out.contains("const __saved = __lg.savedRoomMemberPosition?.(t.id, __others, Math.max(80, Math.round(132 * l)), Math.max(80, Math.round(152 * l)));"),
            "{out}"
        );
        assert!(
            out.contains("const d = __saved ? cl(__saved.left, __saved.top) : cl(o.left + t.left * l, o.top + t.top * l);"),
            "{out}"
        );
        assert!(out.contains("t.left = d.left, t.top = d.top, pt.add(t.id), zi(t)"), "{out}");
        assert!(
            out.contains("async function ml() { if (await tick(), !!ready.value) { for (const [k, v] of members.value.entries()) il(v, k), ep(v); ta(), await tick(), ea() } }"),
            "{out}"
        );
        assert!(out.contains("function __wuwaRoomStorage(event) {"), "{out}");
        assert!(out.contains("window.addEventListener(\"storage\", __wuwaRoomStorage);"), "{out}");
        assert!(
            out.contains("onUnmounted(() => window.removeEventListener(\"storage\", __wuwaRoomStorage));"),
            "{out}"
        );
        assert!(
            out.contains("const __visible = members.value.filter(__m => !__lg.isRoomMemberHidden?.(__m.id));"),
            "{out}"
        );
        assert!(
            out.contains("__lg.arrangeRoomMembers?.(__visible, payload.mode || \"horizontal\", { width: window.innerWidth, height: window.innerHeight }) || [];"),
            "{out}"
        );
        assert!(
            out.contains("__lg.saveRoomMemberPosition?.(__m.id, __c.left, __c.top);"),
            "{out}"
        );
        assert!(out.contains("payload.action === \"arrange\""), "{out}");
        assert!(out.contains("payload.action === \"reset-positions\""), "{out}");
        assert!(
            out.contains("for (const __m of members.value) { pt.delete(__m.id)"),
            "{out}"
        );
        assert!(
            out.contains("if (x && hit(x)) __lg.saveRoomMemberPosition?.(x.id, bounce ? tx : pos.left, bounce ? oy : pos.top);"),
            "{out}"
        );

        let broken = source.replace("xo.value.has(e)", "xo.value.has(q)");
        assert!(patch_room_layout_hooks(&mut rec, &broken, "onUnmounted").is_err());
    }

    #[test]
    fn fn_param_in_scope_ignores_earlier_same_named_function() {
        let text = "function yn(){return 1}\nfunction an(F,T){return F}\nasync function yn(e){\nreturn e\n}\nasync function an(e){\nreturn e\n}\n";
        let comp = vec![
            FunctionMeta {
                name: "yn".into(),
                start: 51,
                end: 83,
                body: 71,
                body_end: 82,
            },
            FunctionMeta {
                name: "an".into(),
                start: 84,
                end: 116,
                body: 104,
                body_end: 115,
            },
        ];
        assert_eq!(
            fn_param_in_scope(text, &comp, &[], "yn").unwrap(),
            Some("e".into())
        );
        assert_eq!(
            fn_param_in_scope(text, &comp, &[], "an").unwrap(),
            Some("e".into())
        );
    }

    #[test]
    fn sync_emotion_patch_skips_mod_assets() {
        let source = "function Mt(w) {\n  z.value && wt(\"re\", {\n    e: w\n  })\n}";
        let mut rec = PatchRecorder::default();
        let out = patch_sync_emotion(&mut rec, source).unwrap();
        assert!(out.contains("if (__lg.isModAsset?.(w)) return;"), "{out}");
    }

    #[test]
    fn patch_log_records_every_hit() {
        let mut rec = PatchRecorder::default();
        assert_eq!(rec.replace_n("a b a", "a", "x", 2).unwrap(), "x b x");
        assert_eq!(rec.entries.len(), 1);
        assert_eq!(rec.entries[0].matches, 2);
        assert_eq!(rec.entries[0].anchor, "a");
    }

    #[test]
    fn monitor_switch_display_patch_exposes_global_hook() {
        let source = r#"
async function Na() {
  await u("set_main_window_monitor_bounds", { payload: {} });
  return true;
}
async function On() {
  if (C) return C;
  C = Na();
  try {
    return await C
  } finally {
    C = null
  }
}
"#;
        let mut rec = PatchRecorder::default();
        let out = patch_monitor_switch_display(&mut rec, source).unwrap();
        assert!(out.contains("window.__WUWA_SWITCH_DISPLAY__ = On;"), "{out}");

        let unrelated = source.replace("set_main_window_monitor_bounds", "other");
        assert!(patch_monitor_switch_display(&mut rec, &unrelated).is_err());
    }
}
