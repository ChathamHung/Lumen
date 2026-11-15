//! Interface languages.
//!
//! Every visible string lives in the table below, so adding a language means
//! adding one column and one match arm - nothing else in the code changes.
//! Missing translations fall back to English at compile time (the macro forces
//! every key to have a value for every language).

use crate::settings::LanguagePref;

/// Languages the interface can be displayed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    /// Resolves the stored preference, detecting the system language when the
    /// preference is "follow system".
    pub fn resolve(preference: LanguagePref) -> Self {
        match preference {
            LanguagePref::English => Self::En,
            LanguagePref::Chinese => Self::Zh,
            LanguagePref::System => system_language(),
        }
    }
}

/// Detects the language of the operating system.
///
/// `LUMEN_LANG=en|zh` overrides the detection, which is handy for testing and
/// for users whose system locale does not match their preferred language.
pub fn system_language() -> Lang {
    if let Ok(value) = std::env::var("LUMEN_LANG") {
        let value = value.trim().to_ascii_lowercase();
        if value.starts_with("zh") {
            return Lang::Zh;
        }
        if value.starts_with("en") {
            return Lang::En;
        }
    }

    let locale = sys_locale::get_locale()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if locale.starts_with("zh") {
        Lang::Zh
    } else {
        Lang::En
    }
}

/// Declares the string table: `Key => ("English", "中文")`.
macro_rules! define_texts {
    ($($variant:ident => ($en:expr, $zh:expr)),* $(,)?) => {
        /// Every user-visible string in the application.
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Text {
            $($variant),*
        }

        impl Text {
            /// Looks this string up in `lang`.
            pub fn get(self, lang: Lang) -> &'static str {
                match self {
                    $(Self::$variant => match lang {
                        Lang::En => $en,
                        Lang::Zh => $zh,
                    }),*
                }
            }
        }
    };
}

/// Shorthand for `text.get(lang)`.
pub fn t(lang: Lang, text: Text) -> &'static str {
    text.get(lang)
}

define_texts! {
    // ----- Window / toolbar -----
    OpenFolder => ("Open folder", "打开文件夹"),
    OpenFile => ("Open file", "打开文件"),
    Previous => ("Previous image  (Left)", "上一张  (←)"),
    Next => ("Next image  (Right)", "下一张  (→)"),
    ZoomOut => ("Zoom out  (-)", "缩小  (-)"),
    ZoomIn => ("Zoom in  (+)", "放大  (+)"),
    FitToWindow => ("Fit to window  (0)", "适应窗口  (0)"),
    ActualSize => ("Actual size  (1)", "实际大小  (1)"),
    RotateLeft => ("Rotate left  (R)", "向左旋转  (R)"),
    RotateRight => ("Rotate right  (Shift+R)", "向右旋转  (Shift+R)"),
    FlipHorizontal => ("Flip horizontally  (H)", "水平翻转  (H)"),
    FlipVertical => ("Flip vertically  (V)", "垂直翻转  (V)"),
    ResetView => ("Reset view  (Ctrl+0)", "重置视图  (Ctrl+0)"),
    StartSlideshow => ("Start slideshow  (Space)", "开始幻灯片  (空格)"),
    StopSlideshow => ("Stop slideshow  (Space)", "停止幻灯片  (空格)"),
    ToggleFilmstrip => ("Toggle filmstrip  (T)", "显示/隐藏缩略图栏  (T)"),
    FileInformation => ("File information  (I)", "文件信息  (I)"),
    SettingsTitle => ("Settings  (Ctrl+,)", "设置  (Ctrl+,)"),
    EnterFullscreen => ("Fullscreen  (F11)", "全屏  (F11)"),
    ExitFullscreen => ("Exit fullscreen  (Esc)", "退出全屏  (Esc)"),

    // ----- Canvas states -----
    DropToOpen => ("Drop to open", "松开以打开"),
    DropHint => ("Drop an image or folder here", "将图片或文件夹拖放到此处"),
    OrOpenFolder => ("or open a folder to start browsing", "或打开一个文件夹开始浏览"),
    NoImagesInFolder => ("No supported images in this folder", "此文件夹中没有可显示的图片"),
    ChooseAnotherFolder => ("Choose another folder", "选择其他文件夹"),
    LoadingImage => ("Loading image…", "正在加载图片…"),
    LoadFailed => ("This image could not be loaded", "无法加载此图片"),
    FolderFailed => ("This folder could not be opened", "无法打开此文件夹"),

    // ----- Status bar / info overlay -----
    Fit => ("Fit", "适应"),
    NoImage => ("No image", "无图片"),
    Dimensions => ("Dimensions", "尺寸"),
    FileSize => ("File size", "文件大小"),
    Modified => ("Modified", "修改时间"),
    Format => ("Format", "格式"),
    Folder => ("Folder", "文件夹"),
    Position => ("Position", "序号"),
    Zoom => ("Zoom", "缩放"),
    ShowInExplorer => ("Show in Explorer", "在资源管理器中显示"),
    OpenContainingFolder => ("Open containing folder", "打开所在文件夹"),
    LoadedIn => ("Loaded in", "加载耗时"),
    Frames => ("Frames", "帧数"),

    // ----- Settings: appearance -----
    Settings => ("Settings", "设置"),
    Appearance => ("Appearance", "外观"),
    Theme => ("Theme", "主题"),
    ThemeSystem => ("System", "跟随系统"),
    ThemeLight => ("Light", "浅色"),
    ThemeDark => ("Dark", "深色"),
    Background => ("Viewer background", "查看器背景"),
    BgCheckerboard => ("Checkerboard", "棋盘格"),
    BgTheme => ("Theme colour", "主题颜色"),
    BgDark => ("Dark", "深色"),
    BgLight => ("Light", "浅色"),
    ShowFilmstrip => ("Show filmstrip", "显示缩略图栏"),
    ShowFilmstripHint => ("A strip of thumbnails above the status bar.", "在状态栏上方显示一排缩略图。"),
    ShowToolbar => ("Show toolbar", "显示工具栏"),
    ShowToolbarHint => ("The bar with the buttons at the top  (Ctrl+T).", "顶部带按钮的工具栏  (Ctrl+T)。"),
    ShowInfo => ("Show file information", "显示文件信息"),
    ShowInfoHint => ("Overlay with resolution, size and date  (I).", "显示分辨率、大小和日期的浮层  (I)。"),
    ShowStatusBar => ("Show status bar", "显示状态栏"),
    ShowStatusBarHint => ("File name, resolution and zoom level.", "文件名、分辨率和缩放比例。"),

    // ----- Settings: viewing -----
    Viewing => ("Viewing", "浏览"),
    OpenFitted => ("Fit images when opened", "打开图片时适应窗口"),
    OpenFittedHint => ("Otherwise new images open at their real size.", "关闭后新图片以实际大小显示。"),
    ZoomSensitivity => ("Wheel zoom sensitivity", "滚轮缩放灵敏度"),
    ZoomSensitivityHint => ("How fast the mouse wheel changes the zoom level.", "鼠标滚轮改变缩放比例的幅度。"),
    PixelGrid => ("Pixel grid", "像素网格"),
    PixelGridHint => (
        "Outline single pixels once the image is magnified past 800%  (G).",
        "放大到 800% 以上时为单个像素描边  (G)。"
    ),
    HoverHighlight => ("Highlight pixel under cursor", "高亮光标下的像素"),
    HoverHighlightHint => (
        "Draw a box around the pixel the status bar reports. Turn it off for a completely clean image.",
        "为状态栏显示的像素画一个方框。关闭后画面完全干净。"
    ),
    PixelGridOff => ("Pixel grid off", "像素网格已关闭"),
    SlideshowInterval => ("Slideshow interval", "幻灯片间隔"),
    LoopSlideshow => ("Repeat slideshow", "循环播放幻灯片"),

    // ----- Settings: library -----
    Library => ("Library", "图库"),
    SortOrder => ("Sort order", "排序方式"),
    SortNameAsc => ("Name  (A → Z)", "名称  (A → Z)"),
    SortNameDesc => ("Name  (Z → A)", "名称  (Z → A)"),
    SortNewest => ("Date  (newest first)", "日期  (最新优先)"),
    SortLargest => ("Size  (largest first)", "大小  (最大优先)"),
    RememberSession => ("Reopen the last folder on startup", "启动时重新打开上次的文件夹"),
    RememberSessionHint => ("Restores the folder and the last viewed image.", "恢复上次的文件夹和最后查看的图片。"),
    ForgetSession => ("Forget last folder", "清除上次的文件夹"),

    // ----- Settings: language -----
    Language => ("Language", "语言"),
    LanguageHint => ("Takes effect immediately.", "立即生效。"),
    LangSystem => ("System", "跟随系统"),
    LangEnglish => ("English", "English"),
    LangChinese => ("Chinese", "中文"),

    // ----- Settings: default viewer -----
    DefaultViewer => ("Default photo viewer", "默认照片查看器"),
    DefaultViewerHint => (
        "Register Lumen as a photo viewer, then pick it in Windows Settings. Windows does not let an application take over file associations silently, so the last step has to be confirmed by you.",
        "将 Lumen 注册为照片查看器，然后在 Windows 设置中选择它。Windows 不允许应用静默接管文件关联，因此最后一步需要你手动确认。"
    ),
    RegisterDefault => ("Register and open Windows Settings", "注册并打开 Windows 设置"),
    UnregisterDefault => ("Remove registration", "移除注册"),
    Registered => ("Registered for JPG, PNG, GIF, BMP, WebP, TIFF and ICO.", "已注册 JPG、PNG、GIF、BMP、WebP、TIFF 和 ICO。"),
    NotRegistered => ("Not registered yet.", "尚未注册。"),
    WindowsOnly => ("Only available on Windows.", "仅在 Windows 上可用。"),

    // ----- Settings: configuration file -----
    ConfigFile => ("Settings file", "设置文件"),
    ConfigFileHint => ("Everything on this page is stored in this file.", "此页面的所有选项都保存在这个文件中。"),
    ExportSettings => ("Export…", "导出…"),
    ImportSettings => ("Import…", "导入…"),
    ResetDefaults => ("Reset to defaults", "恢复默认"),
    OpenFolderAction => ("Open folder", "打开文件夹"),

    // ----- Settings: about -----
    About => ("About", "关于"),
    Version => ("Version", "版本"),
    StartupTime => ("Startup time", "启动时间"),
    Renderer => ("Renderer", "渲染器"),
    Repository => ("Source code", "源代码"),
    SupportedFormats => ("Supported formats", "支持的格式"),
    Shortcuts => ("Keyboard shortcuts", "键盘快捷键"),
    AboutBlurb => (
        "A fast, portable photo viewer built with Rust and egui. MIT licensed.",
        "使用 Rust 和 egui 构建的快速、便携的照片查看器。MIT 许可协议。"
    ),

    // ----- Shortcuts window -----
    ShortcutsTitle => ("Keyboard shortcuts", "键盘快捷键"),
    ScPrevNext => ("Previous / next image", "上一张 / 下一张"),
    ScFirstLast => ("First / last image", "第一张 / 最后一张"),
    ScZoom => ("Zoom in / out", "放大 / 缩小"),
    ScFit => ("Fit to window", "适应窗口"),
    ScActual => ("Actual size (100%)", "实际大小 (100%)"),
    ScRotate => ("Rotate left / right", "向左 / 向右旋转"),
    ScFlip => ("Flip horizontally / vertically", "水平 / 垂直翻转"),
    ScResetView => ("Reset rotation, flip and zoom", "重置旋转、翻转和缩放"),
    ScSlideshow => ("Start / stop slideshow", "开始 / 停止幻灯片"),
    ScInfo => ("Toggle file information", "显示/隐藏文件信息"),
    ScFilmstrip => ("Toggle filmstrip", "显示/隐藏缩略图栏"),
    ScToolbar => ("Toggle toolbar", "显示/隐藏工具栏"),
    ScPixelGrid => ("Toggle pixel grid", "显示/隐藏像素网格"),
    ScFullscreen => ("Toggle fullscreen", "切换全屏"),
    ScSettings => ("Open settings", "打开设置"),
    ScShortcuts => ("Show this shortcut list", "显示本快捷键列表"),
    ScOpen => ("Open folder / open file", "打开文件夹 / 打开文件"),
    ScClose => ("Close dialog or leave fullscreen", "关闭对话框或退出全屏"),

    // ----- Messages -----
    SettingsExported => ("Settings exported", "设置已导出"),
    SettingsImported => ("Settings imported", "设置已导入"),
    SettingsReset => ("Settings reset to defaults", "设置已恢复默认"),
    ExportFailed => ("Could not export the settings", "无法导出设置"),
    ImportFailed => ("Could not import the settings", "无法导入设置"),
    RegistrationDone => ("Registered. Choose Lumen in Windows Settings.", "注册完成，请在 Windows 设置中选择 Lumen。"),
    RegistrationFailed => ("Registration failed", "注册失败"),
    RegistrationRemoved => ("Registration removed", "已移除注册"),
    NoMoreImages => ("End of folder", "已到文件夹末尾"),
    Unknown => ("Unknown", "未知"),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_a_language_for_this_machine() {
        // Whatever the machine is set to, resolution must land on a language
        // that has a full translation table.
        let lang = Lang::resolve(LanguagePref::System);
        println!("detected system language: {lang:?}");
        assert!(matches!(lang, Lang::En | Lang::Zh));
    }

    #[test]
    fn explicit_preferences_win_over_the_system() {
        assert_eq!(Lang::resolve(LanguagePref::English), Lang::En);
        assert_eq!(Lang::resolve(LanguagePref::Chinese), Lang::Zh);
    }
}
