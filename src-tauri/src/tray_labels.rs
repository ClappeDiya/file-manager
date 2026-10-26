//! The tray menu's texts in the app's languages (Settings → Appearance →
//! Language). The web view tells the shell its language with
//! `set_ui_language`; the tray is relabelled at once and the mounts submenu
//! uses it when it is rebuilt. Unknown languages use English.

use std::sync::RwLock;

/// The tray menu's fixed texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayLabels {
    pub open: &'static str,
    pub new_window: &'static str,
    pub backups: &'static str,
    pub mounts: &'static str,
    pub no_mounts: &'static str,
    pub unmount_all: &'static str,
    pub run_all: &'static str,
    pub pause: &'static str,
    pub resume: &'static str,
    pub lock: &'static str,
    pub quit: &'static str,
}

const EN: TrayLabels = TrayLabels {
    open: "Open UFOP",
    new_window: "New window",
    backups: "Backups…",
    mounts: "Mounts",
    no_mounts: "No saved mounts",
    unmount_all: "Unmount all",
    run_all: "Back up all now",
    pause: "Pause backups for 1 hour",
    resume: "Resume backups",
    lock: "Lock UFOP",
    quit: "Quit",
};

const ES: TrayLabels = TrayLabels {
    open: "Abrir UFOP",
    new_window: "Ventana nueva",
    backups: "Copias de seguridad…",
    mounts: "Unidades montadas",
    no_mounts: "No hay unidades guardadas",
    unmount_all: "Desmontar todas",
    run_all: "Hacer todas las copias ahora",
    pause: "Pausar las copias 1 hora",
    resume: "Reanudar las copias",
    lock: "Bloquear UFOP",
    quit: "Salir",
};

const FR: TrayLabels = TrayLabels {
    open: "Ouvrir UFOP",
    new_window: "Nouvelle fenêtre",
    backups: "Sauvegardes…",
    mounts: "Lecteurs montés",
    no_mounts: "Aucun lecteur enregistré",
    unmount_all: "Tout démonter",
    run_all: "Tout sauvegarder maintenant",
    pause: "Suspendre les sauvegardes 1 heure",
    resume: "Reprendre les sauvegardes",
    lock: "Verrouiller UFOP",
    quit: "Quitter",
};

const DE: TrayLabels = TrayLabels {
    open: "UFOP öffnen",
    new_window: "Neues Fenster",
    backups: "Sicherungen…",
    mounts: "Laufwerke",
    no_mounts: "Keine gespeicherten Laufwerke",
    unmount_all: "Alle trennen",
    run_all: "Jetzt alles sichern",
    pause: "Sicherungen 1 Stunde pausieren",
    resume: "Sicherungen fortsetzen",
    lock: "UFOP sperren",
    quit: "Beenden",
};

const PT: TrayLabels = TrayLabels {
    open: "Abrir o UFOP",
    new_window: "Nova janela",
    backups: "Backups…",
    mounts: "Unidades montadas",
    no_mounts: "Nenhuma unidade salva",
    unmount_all: "Desmontar todas",
    run_all: "Fazer todos os backups agora",
    pause: "Pausar backups por 1 hora",
    resume: "Retomar backups",
    lock: "Bloquear o UFOP",
    quit: "Sair",
};

const JA: TrayLabels = TrayLabels {
    open: "UFOP を開く",
    new_window: "新しいウィンドウ",
    backups: "バックアップ…",
    mounts: "マウント",
    no_mounts: "保存されたマウントはありません",
    unmount_all: "すべてアンマウント",
    run_all: "今すぐすべてバックアップ",
    pause: "バックアップを 1 時間停止",
    resume: "バックアップを再開",
    lock: "UFOP をロック",
    quit: "終了",
};

const ZH: TrayLabels = TrayLabels {
    open: "打开 UFOP",
    new_window: "新建窗口",
    backups: "备份…",
    mounts: "挂载",
    no_mounts: "没有已保存的挂载",
    unmount_all: "全部卸载",
    run_all: "立即全部备份",
    pause: "暂停备份 1 小时",
    resume: "恢复备份",
    lock: "锁定 UFOP",
    quit: "退出",
};

const KO: TrayLabels = TrayLabels {
    open: "UFOP 열기",
    new_window: "새 창",
    backups: "백업…",
    mounts: "마운트",
    no_mounts: "저장된 마운트 없음",
    unmount_all: "모두 마운트 해제",
    run_all: "지금 모두 백업",
    pause: "백업 1시간 일시 중지",
    resume: "백업 다시 시작",
    lock: "UFOP 잠그기",
    quit: "종료",
};

/// The languages the app has translations for (as in `src/lib/i18n.ts`).
pub const SUPPORTED: [&str; 8] = ["en", "es", "fr", "de", "pt", "ja", "zh", "ko"];

/// The texts for a language code (`de`, `de-DE`, `pt_BR`…); English otherwise.
pub fn labels(locale: &str) -> TrayLabels {
    let primary = locale.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    match primary.as_str() {
        "es" => ES,
        "fr" => FR,
        "de" => DE,
        "pt" => PT,
        "ja" => JA,
        "zh" => ZH,
        "ko" => KO,
        _ => EN,
    }
}

static CURRENT: RwLock<String> = RwLock::new(String::new());

/// Remember the app's language; `false` when it is not one of the supported ones.
pub fn set_current(locale: &str) -> bool {
    let primary = locale.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    if !SUPPORTED.contains(&primary.as_str()) {
        return false;
    }
    if let Ok(mut c) = CURRENT.write() {
        *c = primary;
    }
    true
}

/// The texts for the app's current language.
pub fn current() -> TrayLabels {
    labels(&CURRENT.read().map(|c| c.clone()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_its_own_texts_and_unknown_ones_get_english() {
        let all: Vec<TrayLabels> = SUPPORTED.iter().map(|l| labels(l)).collect();
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.quit, b.quit, "two languages share the Quit text");
            }
        }
        assert_eq!(labels("de-DE"), DE);
        assert_eq!(labels("pt_BR"), PT);
        assert_eq!(labels("ZH"), ZH);
        assert_eq!(labels("xx"), EN);
        assert_eq!(labels(""), EN);
    }

    #[test]
    fn the_current_language_is_kept_and_unknown_ones_are_refused() {
        assert!(set_current("fr-CA"));
        assert_eq!(current(), FR);
        assert!(!set_current("tlh"));
        assert_eq!(current(), FR);
        assert!(set_current("en"));
        assert_eq!(current(), EN);
    }
}
