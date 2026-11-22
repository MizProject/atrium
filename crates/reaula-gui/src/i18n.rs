//! Lightweight multi-language text table for onboarding, shell chrome, and keyboard hub.

use crate::config::AppLanguage;
use crate::state::NavSection;

pub struct UiText {
    pub nav_setup: &'static str,
    pub nav_main: &'static str,
    pub nav_calibration: &'static str,
    pub nav_color_light: &'static str,
    pub nav_hotkeys: &'static str,
    pub nav_diagnostics: &'static str,

    pub guide_help_btn: &'static str,
    pub live_travel: &'static str,
    pub zero_btn: &'static str,
    pub reconnect_btn: &'static str,
    pub daemon_offline: &'static str,

    pub step_welcome: &'static str,
    pub step_disclaimer: &'static str,
    pub step_system: &'static str,
    pub step_hardware: &'static str,
    pub step_finish: &'static str,

    pub setup_title: &'static str,
    pub setup_subtitle: &'static str,
    pub choose_language: &'static str,
    pub choose_theme: &'static str,
    pub dark_mode_label: &'static str,
    pub light_mode_label: &'static str,

    pub disclaimer_heading: &'static str,
    pub disclaimer_privacy: &'static str,
    pub disclaimer_liability: &'static str,
    pub disclaimer_opensource: &'static str,
    pub disclaimer_issues: &'static str,
    pub disclaimer_accept_chk: &'static str,
    pub disclaimer_must_accept: &'static str,

    pub system_heading: &'static str,
    pub system_subtitle: &'static str,
    pub refresh_checks: &'static str,
    pub reconnect_daemon: &'static str,

    pub hardware_heading: &'static str,
    pub supported_hw_tab: &'static str,
    pub unsupported_hw_tab: &'static str,
    pub select_supported_desc: &'static str,
    pub unsupported_desc: &'static str,
    pub select_hid_label: &'static str,
    pub model_hint_label: &'static str,
    pub vendor_probe_chk: &'static str,
    pub run_probe_btn: &'static str,
    pub copy_json_btn: &'static str,
    pub save_json_btn: &'static str,
    pub open_issue_btn: &'static str,
    pub recovery_warning_title: &'static str,
    pub recovery_warning_body: &'static str,
    pub dev_followup_notice: &'static str,

    pub summary_heading: &'static str,
    pub back_btn: &'static str,
    pub next_btn: &'static str,
    pub finish_setup_btn: &'static str,

    pub my_keyboards_title: &'static str,
    pub add_keyboard_btn: &'static str,
    pub setup_wizard_btn: &'static str,
    pub activate_btn: &'static str,
    pub active_badge: &'static str,
    pub remove_btn: &'static str,
    pub supported_badge: &'static str,
    pub unsupported_badge: &'static str,
    pub no_keyboards_empty: &'static str,
}

impl UiText {
    pub const fn for_lang(lang: AppLanguage) -> &'static Self {
        match lang {
            AppLanguage::En => &TEXT_EN,
            AppLanguage::Id => &TEXT_ID,
            AppLanguage::Es => &TEXT_ES,
            AppLanguage::Ja => &TEXT_JA,
            AppLanguage::ZhCn => &TEXT_ZH_CN,
        }
    }

    pub const fn nav_label(&self, sec: NavSection) -> &'static str {
        match sec {
            NavSection::Setup => self.nav_setup,
            NavSection::Main => self.nav_main,
            NavSection::Calibration => self.nav_calibration,
            NavSection::ColorLight => self.nav_color_light,
            NavSection::Hotkeys => self.nav_hotkeys,
            NavSection::Diagnostics => self.nav_diagnostics,
        }
    }
}

const TEXT_EN: UiText = UiText {
    nav_setup: "Setup",
    nav_main: "Main",
    nav_calibration: "Calibration",
    nav_color_light: "Color & Light",
    nav_hotkeys: "Hotkeys",
    nav_diagnostics: "Diagnostics",

    guide_help_btn: "Guide & Help\u{2026}",
    live_travel: "Live Travel",
    zero_btn: "Zero",
    reconnect_btn: "Reconnect",
    daemon_offline: "Daemon Offline",

    step_welcome: "Welcome",
    step_disclaimer: "Disclaimer",
    step_system: "Checks",
    step_hardware: "Hardware",
    step_finish: "Finish",

    setup_title: "Welcome to ReAULA",
    setup_subtitle: "Configure language, theme, system permissions, daemon IPC, and your keyboard before entering the workbench.",
    choose_language: "Interface Language",
    choose_theme: "Appearance Theme",
    dark_mode_label: "Dark Mode",
    light_mode_label: "Light Mode",

    disclaimer_heading: "MizProject Open-Source Disclaimer & Privacy Notice",
    disclaimer_privacy: "1. Privacy & Security Audit: MizProject does not take any secret or personal information from the user. Cybersecurity experts and users are free to explore and dissect the source code.",
    disclaimer_liability: "2. Liability: MizProject is not liable for damages that happen during, while, or not using this software.",
    disclaimer_opensource: "3. Open Source: MizProject wants the code and software to be open source and distributed freely as possible.",
    disclaimer_issues: "4. Support & Issue Tracking: Any support submissions should be uploaded on GitHub Issues with an appropriate request for support.",
    disclaimer_accept_chk: "I have read and understand the MizProject disclaimer above.",
    disclaimer_must_accept: "Please acknowledge the disclaimer in Step 2 before finishing setup.",

    system_heading: "Permissions & Daemon Connection",
    system_subtitle: "Verify HID device permissions (/dev/hidraw*), udev rules, /dev/uinput, and reaulad daemon connectivity.",
    refresh_checks: "Refresh Checks",
    reconnect_daemon: "Reconnect Daemon",

    hardware_heading: "Keyboard Selection & Unsupported HID Prober",
    supported_hw_tab: "Supported Keyboard",
    unsupported_hw_tab: "Unsupported Keyboard / Probe",
    select_supported_desc: "Select a supported keyboard driver compiled into ReAULA, or switch to the Unsupported tab to inspect an unrecognized AULA model in 3D and collect capability JSON.",
    unsupported_desc: "Preview the generic 3D keyboard model, pick your HID interface below, and run capability checks. Review and save the sanitized JSON to submit an issue.",
    select_hid_label: "Target HID Interface (/dev/hidraw*)",
    model_hint_label: "Keyboard Model Name (e.g. AULA F75, Hero 68 HE)",
    vendor_probe_chk: "Allow safe read-only vendor queries (FW / info / profile)",
    run_probe_btn: "Probe Capabilities & Generate JSON",
    copy_json_btn: "Copy JSON",
    save_json_btn: "Save JSON File",
    open_issue_btn: "Open GitHub Support Issue",
    recovery_warning_title: "Hardware Recovery Notice (Fn + Esc 10s)",
    recovery_warning_body: "If the keyboard acts weird during probing, unplug and replug the USB cable, and then try resetting if possible by holding Fn + Esc for 10 seconds.",
    dev_followup_notice: "The developer may respond to your issue request. Please also try to respond back promptly so we can quickly get your AULA keyboard supported.",

    summary_heading: "Ready for Main Screen",
    back_btn: "Back",
    next_btn: "Next",
    finish_setup_btn: "Finish Setup & Open Main",

    my_keyboards_title: "My Keyboards",
    add_keyboard_btn: "Add Keyboard\u{2026}",
    setup_wizard_btn: "Setup Wizard\u{2026}",
    activate_btn: "Activate",
    active_badge: "Active",
    remove_btn: "Remove",
    supported_badge: "Supported",
    unsupported_badge: "Unsupported",
    no_keyboards_empty: "No keyboards configured yet. Add a keyboard to begin.",
};

const TEXT_ID: UiText = UiText {
    nav_setup: "Pengaturan",
    nav_main: "Utama",
    nav_calibration: "Kalibrasi",
    nav_color_light: "Warna & Lampu",
    nav_hotkeys: "Pintasan",
    nav_diagnostics: "Diagnostik",

    guide_help_btn: "Panduan\u{2026}",
    live_travel: "Travel Langsung",
    zero_btn: "Nolkan",
    reconnect_btn: "Hubung Ulang",
    daemon_offline: "Daemon Luring",

    step_welcome: "Selamat Datang",
    step_disclaimer: "Penafian",
    step_system: "Pemeriksaan",
    step_hardware: "Perangkat",
    step_finish: "Selesai",

    setup_title: "Selamat Datang di ReAULA",
    setup_subtitle: "Atur bahasa, tema, izin sistem, koneksi daemon, dan keyboard Anda sebelum masuk ke layar utama.",
    choose_language: "Bahasa Antarmuka",
    choose_theme: "Tema Tampilan",
    dark_mode_label: "Mode Gelap",
    light_mode_label: "Mode Terang",

    disclaimer_heading: "Penafian Sumber Terbuka & Privasi MizProject",
    disclaimer_privacy: "1. Privasi & Audit Keamanan: MizProject tidak mengambil informasi rahasia dari pengguna. Pakar keamanan siber bebas memeriksa dan membedah kode sumbernya.",
    disclaimer_liability: "2. Tanggung Jawab: MizProject tidak bertanggung jawab atas kerusakan yang terjadi saat, selama, atau tidak menggunakan perangkat lunak ini.",
    disclaimer_opensource: "3. Sumber Terbuka: MizProject ingin kode dan perangkat lunak ini tetap open source dan didistribusikan sebebas mungkin.",
    disclaimer_issues: "4. Dukungan & Laporan: Semua permintaan dukungan harus diunggah melalui GitHub Issues dengan permintaan dukungan yang sesuai.",
    disclaimer_accept_chk: "Saya telah membaca dan memahami penafian MizProject di atas.",
    disclaimer_must_accept: "Harap setujui penafian di langkah sebelumnya untuk menyelesaikan pengaturan.",

    system_heading: "Izin Sistem & Koneksi Daemon",
    system_subtitle: "Periksa izin perangkat HID (/dev/hidraw*), aturan udev, /dev/uinput, dan status daemon reaulad.",
    refresh_checks: "Segarkan",
    reconnect_daemon: "Hubungkan Daemon",

    hardware_heading: "Pilihan Keyboard & Probe HID Belum Didukung",
    supported_hw_tab: "Keyboard Didukung",
    unsupported_hw_tab: "Belum Didukung / Probe",
    select_supported_desc: "Pilih driver keyboard yang didukung ReAULA, atau buka tab Belum Didukung untuk melihat model 3D umum dan memindai HID sebagai JSON.",
    unsupported_desc: "Lihat pratinjau keyboard 3D generik, pilih antarmuka HID Anda, jalankan pemeriksaan kapabilitas, dan simpan data JSON untuk dikirim ke GitHub Issues.",
    select_hid_label: "Target Antarmuka HID (/dev/hidraw*)",
    model_hint_label: "Nama Model Keyboard (mis. AULA F75)",
    vendor_probe_chk: "Izinkan kueri vendor baca-saja yang aman",
    run_probe_btn: "Pindai Kapabilitas & Buat JSON",
    copy_json_btn: "Salin JSON",
    save_json_btn: "Simpan File JSON",
    open_issue_btn: "Buka Form Issue GitHub",
    recovery_warning_title: "Peringatan Pemulihan (Tahan Fn + Esc 10 Detik)",
    recovery_warning_body: "Jika keyboard berperilaku aneh selama pemindaian, cabut lalu colok ulang kabel USB, lalu coba reset dengan menahan Fn + Esc selama 10 detik.",
    dev_followup_notice: "Pengembang dapat membalas permintaan Anda. Harap balas juga secara aktif agar keyboard AULA Anda dapat segera didukung.",

    summary_heading: "Siap Masuk ke Layar Utama",
    back_btn: "Kembali",
    next_btn: "Lanjut",
    finish_setup_btn: "Selesai & Buka Layar Utama",

    my_keyboards_title: "Keyboard Saya",
    add_keyboard_btn: "Tambah Keyboard\u{2026}",
    setup_wizard_btn: "Panduan Awal\u{2026}",
    activate_btn: "Aktifkan",
    active_badge: "Aktif",
    remove_btn: "Hapus",
    supported_badge: "Didukung",
    unsupported_badge: "Belum Didukung",
    no_keyboards_empty: "Belum ada keyboard dikonfigurasi. Tambahkan keyboard untuk memulai.",
};

const TEXT_ES: UiText = UiText {
    nav_setup: "Configurar",
    nav_main: "Principal",
    nav_calibration: "Calibración",
    nav_color_light: "Color y Luz",
    nav_hotkeys: "Atajos",
    nav_diagnostics: "Diagnóstico",

    guide_help_btn: "Guía y Ayuda\u{2026}",
    live_travel: "Recorrido Vivo",
    zero_btn: "Poner a 0",
    reconnect_btn: "Reconectar",
    daemon_offline: "Daemon Desconectado",

    step_welcome: "Bienvenida",
    step_disclaimer: "Aviso Legal",
    step_system: "Permisos",
    step_hardware: "Hardware",
    step_finish: "Finalizar",

    setup_title: "Bienvenido a ReAULA",
    setup_subtitle: "Configura el idioma, tema, permisos del sistema, demonio IPC y tu teclado antes de usar la aplicación.",
    choose_language: "Idioma de Interfaz",
    choose_theme: "Tema Visual",
    dark_mode_label: "Modo Oscuro",
    light_mode_label: "Modo Claro",

    disclaimer_heading: "Aviso de Código Abierto y Privacidad de MizProject",
    disclaimer_privacy: "1. Privacidad y Auditoría: MizProject no recopila información secreta del usuario; expertos en ciberseguridad pueden explorar y analizar libremente el código fuente.",
    disclaimer_liability: "2. Responsabilidad: MizProject no se hace responsable de daños ocurridos durante, mientras o sin usar este software.",
    disclaimer_opensource: "3. Código Abierto: MizProject desea que el código y el software sean de código abierto y se distribuyan lo más libremente posible.",
    disclaimer_issues: "4. Soporte en Issues: Cualquier solicitud de soporte debe subirse a GitHub Issues con la plantilla adecuada.",
    disclaimer_accept_chk: "He leído y comprendo el aviso de MizProject anterior.",
    disclaimer_must_accept: "Acepta el aviso legal antes de finalizar la configuración.",

    system_heading: "Permisos y Conexión del Daemon",
    system_subtitle: "Comprueba permisos de /dev/hidraw*, reglas udev, /dev/uinput y la conexión con reaulad.",
    refresh_checks: "Actualizar",
    reconnect_daemon: "Reconectar Daemon",

    hardware_heading: "Selección de Teclado y Sonda HID",
    supported_hw_tab: "Teclado Compatible",
    unsupported_hw_tab: "No Compatible / Sonda",
    select_supported_desc: "Elige un teclado compatible o usa la pestaña No Compatible para previsualizar en 3D y exportar sus datos HID en JSON.",
    unsupported_desc: "Previsualiza el modelo 3D genérico, selecciona tu interfaz HID, sondea capacidades y guarda el JSON para abrir un issue.",
    select_hid_label: "Interfaz HID (/dev/hidraw*)",
    model_hint_label: "Nombre del Modelo (ej. AULA F75)",
    vendor_probe_chk: "Permitir consultas seguras de solo lectura",
    run_probe_btn: "Sondear Capacidades y Generar JSON",
    copy_json_btn: "Copiar JSON",
    save_json_btn: "Guardar Archivo JSON",
    open_issue_btn: "Abrir Issue en GitHub",
    recovery_warning_title: "Aviso de Recuperación (Fn + Esc 10s)",
    recovery_warning_body: "Si el teclado se comporta de forma extraña al sondear, desconecta y vuelve a conectar el cable USB e intenta restablecer manteniendo Fn + Esc durante 10 segundos.",
    dev_followup_notice: "El desarrollador puede responder a tu solicitud; por favor responde pronto para dar soporte rápido a tu teclado AULA.",

    summary_heading: "Listo para el Panel Principal",
    back_btn: "Atrás",
    next_btn: "Siguiente",
    finish_setup_btn: "Finalizar y Abrir Principal",

    my_keyboards_title: "Mis Teclados",
    add_keyboard_btn: "Añadir Teclado\u{2026}",
    setup_wizard_btn: "Asistente Inicial\u{2026}",
    activate_btn: "Activar",
    active_badge: "Activo",
    remove_btn: "Quitar",
    supported_badge: "Compatible",
    unsupported_badge: "No Compatible",
    no_keyboards_empty: "No hay teclados configurados aún. Añade un teclado para empezar.",
};

const TEXT_JA: UiText = UiText {
    nav_setup: "初期設定",
    nav_main: "メイン",
    nav_calibration: "校正",
    nav_color_light: "照明設定",
    nav_hotkeys: "キー割り当て",
    nav_diagnostics: "診断",

    guide_help_btn: "ガイド\u{2026}",
    live_travel: "リアルタイム変位",
    zero_btn: "ゼロ復帰",
    reconnect_btn: "再接続",
    daemon_offline: "デーモン未接続",

    step_welcome: "ようこそ",
    step_disclaimer: "免責事項",
    step_system: "権限確認",
    step_hardware: "機器選択",
    step_finish: "完了",

    setup_title: "ReAULA へようこそ",
    setup_subtitle: "言語・テーマ・システム権限・デーモン接続・キーボード構成を順に設定します。",
    choose_language: "表示言語",
    choose_theme: "カラーテーマ",
    dark_mode_label: "ダークモード",
    light_mode_label: "ライトモード",

    disclaimer_heading: "MizProject オープンソース免責事項・プライバシー通知",
    disclaimer_privacy: "1. プライバシーと検証: MizProject はユーザーの機密情報を一切収集しません。セキュリティ専門家も自由にソースコードを検証できます。",
    disclaimer_liability: "2. 免責: 本ソフトウェアの使用中・使用前後・不使用を問わず生じた損害について、MizProject は責任を負いません。",
    disclaimer_opensource: "3. オープンソース: MizProject は可能な限り自由にコードとソフトをオープンソースで配布します。",
    disclaimer_issues: "4. サポート窓口: サポート依頼は GitHub Issues へ適切な申請として提出してください。",
    disclaimer_accept_chk: "上記の MizProject 免責事項を読み理解しました。",
    disclaimer_must_accept: "設定を完了するには免責事項への同意が必要です。",

    system_heading: "権限とデーモン接続状態",
    system_subtitle: "/dev/hidraw*、udev ルール、/dev/uinput、および reaulad デーモン接続を確認します。",
    refresh_checks: "再確認",
    reconnect_daemon: "デーモン再接続",

    hardware_heading: "キーボード選択・未対応デバイス HID プローブ",
    supported_hw_tab: "対応キーボード",
    unsupported_hw_tab: "未対応 / プローブ",
    select_supported_desc: "対応機種を選択するか、未対応タブから汎用3Dモデル表示と HID 調査 JSON 出力を行えます。",
    unsupported_desc: "汎用 3D キーボードを確認し、対象の HID デバイスを選択して能力調査と JSON 出力を実行します。",
    select_hid_label: "対象 HID デバイス (/dev/hidraw*)",
    model_hint_label: "キーボード型番メモ (例: AULA F75)",
    vendor_probe_chk: "安全な読み取り専用ベンダークエリを許可",
    run_probe_btn: "HID プローブ実行 & JSON 生成",
    copy_json_btn: "JSON をコピー",
    save_json_btn: "JSON を保存",
    open_issue_btn: "GitHub Issue フォームを開く",
    recovery_warning_title: "復旧手順 (Fn + Esc 10秒長押し)",
    recovery_warning_body: "調査中に動作が不安定になった場合は USB を抜き差しし、可能であれば Fn + Esc を 10 秒間長押ししてリセットを試してください。",
    dev_followup_notice: "開発者が Issue に返信する場合があります。迅速に対応できるようお返事へのご協力をお願いします。",

    summary_heading: "メイン画面の準備完了",
    back_btn: "戻る",
    next_btn: "次へ",
    finish_setup_btn: "設定を完了して開始",

    my_keyboards_title: "登録済みキーボード",
    add_keyboard_btn: "キーボード追加\u{2026}",
    setup_wizard_btn: "初期設定ウィザード\u{2026}",
    activate_btn: "選択",
    active_badge: "使用中",
    remove_btn: "削除",
    supported_badge: "対応済み",
    unsupported_badge: "未対応",
    no_keyboards_empty: "キーボードが登録されていません。キーボードを追加してください。",
};

const TEXT_ZH_CN: UiText = UiText {
    nav_setup: "初始向导",
    nav_main: "主页",
    nav_calibration: "按键校准",
    nav_color_light: "灯光与色彩",
    nav_hotkeys: "快捷键",
    nav_diagnostics: "诊断",

    guide_help_btn: "指南与帮助\u{2026}",
    live_travel: "实时行程",
    zero_btn: "归零",
    reconnect_btn: "重连",
    daemon_offline: "守护进程未连接",

    step_welcome: "欢迎",
    step_disclaimer: "免责声明",
    step_system: "权限检测",
    step_hardware: "硬件探测",
    step_finish: "完成",

    setup_title: "欢迎使用 ReAULA",
    setup_subtitle: "在进入主工作区之前，按步骤完成语言、主题、系统权限、守护进程与键盘选择。",
    choose_language: "界面语言",
    choose_theme: "外观主题",
    dark_mode_label: "深色模式",
    light_mode_label: "浅色模式",

    disclaimer_heading: "MizProject 开源免责声明与隐私说明",
    disclaimer_privacy: "1. 隐私与审计: MizProject 不会收集任何用户机密数据，网络安全专家可自由审查和剖析全部源代码。",
    disclaimer_liability: "2. 免责条款: 对于使用、未使用或使用本软件期间发生的任何损害，MizProject 概不承担责任。",
    disclaimer_opensource: "3. 开源宗旨: MizProject 致力于保持软件开源并尽可能自由地分发。",
    disclaimer_issues: "4. 支持渠道: 所有支持请求请按模板规范提交至 GitHub Issues。",
    disclaimer_accept_chk: "我已阅读并理解上述 MizProject 声明。",
    disclaimer_must_accept: "请先确认第 2 步中的声明再完成设置。",

    system_heading: "权限与守护进程状态检查",
    system_subtitle: "检测 /dev/hidraw* 权限、udev 规则、/dev/uinput 以及 reaulad 守护进程连接。",
    refresh_checks: "重新检测",
    reconnect_daemon: "重连守护进程",

    hardware_heading: "已支持键盘或未知设备 HID 探测",
    supported_hw_tab: "已支持键盘",
    unsupported_hw_tab: "未知设备 / HID 探测",
    select_supported_desc: "选择内置支持的键盘，或切换至未知设备标签以查看通用 3D 模型并导出口味探测 JSON。",
    unsupported_desc: "预览通用 3D 键盘模型，选择对应 HID 接口并运行安全探测，导出 JSON 后前往 GitHub Issues 提交。",
    select_hid_label: "选择 HID 接口 (/dev/hidraw*)",
    model_hint_label: "键盘型号备注 (如 AULA F75)",
    vendor_probe_chk: "允许安全的只读厂商查询 (固件/信息/配置)",
    run_probe_btn: "探测设备能力并生成 JSON",
    copy_json_btn: "复制 JSON",
    save_json_btn: "保存 JSON 文件",
    open_issue_btn: "打开 GitHub 提交表单",
    recovery_warning_title: "异常恢复提示 (长按 Fn + Esc 10 秒)",
    recovery_warning_body: "如果探测过程中键盘出现异常，请先拔插 USB 数据线，然后长按 Fn + Esc 约 10 秒尝试恢复出厂设置。",
    dev_followup_notice: "开发者可能会回复您的 Issue 请求，请及时留意并回复以便更快为您的 AULA 键盘适配支持。",

    summary_heading: "准备就绪",
    back_btn: "上一步",
    next_btn: "下一步",
    finish_setup_btn: "完成向导并进入主页",

    my_keyboards_title: "我的键盘",
    add_keyboard_btn: "添加键盘\u{2026}",
    setup_wizard_btn: "初始设置向导\u{2026}",
    activate_btn: "启用",
    active_badge: "当前",
    remove_btn: "移除",
    supported_badge: "已支持",
    unsupported_badge: "未支持",
    no_keyboards_empty: "尚未配置键盘，请先添加一个键盘。",
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_translations_present_and_free_of_em_dash() {
        for lang in AppLanguage::ALL {
            let t = UiText::for_lang(lang);
            let fields = [
                t.nav_setup,
                t.nav_main,
                t.nav_calibration,
                t.nav_color_light,
                t.nav_hotkeys,
                t.nav_diagnostics,
                t.setup_title,
                t.disclaimer_privacy,
                t.disclaimer_liability,
                t.disclaimer_opensource,
                t.disclaimer_issues,
                t.recovery_warning_body,
                t.dev_followup_notice,
                t.finish_setup_btn,
                t.my_keyboards_title,
                t.add_keyboard_btn,
                t.remove_btn,
            ];
            for val in fields {
                assert!(!val.trim().is_empty(), "empty field for {lang:?}");
                assert!(!val.contains('\u{2014}'), "found forbidden em-dash in {lang:?}: {val}");
            }
        }
    }
}
