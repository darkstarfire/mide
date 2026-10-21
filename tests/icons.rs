//! Busca de ícones com o tema do mide: num processo próprio, porque as pastas de
//! dados (`XDG_DATA_HOME`) são lidas uma vez.

use std::fs;

/// Ícone simples (um quadrado) para o tema de teste.
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><rect width="64" height="64" fill="#123456"/></svg>"##;

#[test]
fn tema_do_mide_vem_primeiro_e_os_apps_do_mide_tem_reserva() {
    let dir = std::env::temp_dir().join(format!("mide-icons-test-{}", std::process::id()));
    let theme = dir.join("icons/mide");
    fs::create_dir_all(theme.join("apps/scalable")).unwrap();
    fs::write(
        theme.join("index.theme"),
        "[Icon Theme]\nName=mide\nInherits=hicolor\nDirectories=apps/scalable\n\n[apps/scalable]\nSize=64\nMinSize=16\nMaxSize=512\nType=Scalable\n",
    )
    .unwrap();
    fs::write(theme.join("apps/scalable/mide-terminal.svg"), SVG).unwrap();
    // SAFETY: este teste roda sozinho no processo (arquivo de teste próprio) e antes
    // de qualquer busca de ícone.
    unsafe { std::env::set_var("XDG_DATA_HOME", &dir) };

    assert!(mide::icons::mide_theme_installed());
    let icon = mide::icons::load_icon("mide-terminal", 64).expect("ícone do tema do mide");
    // A cor do quadrado de teste: veio do tema do mide.
    let px = icon.pixel(32, 32).unwrap();
    assert_eq!((px.red(), px.green(), px.blue()), (0x12, 0x34, 0x56));
    // Sem o ícone no tema, os apps do mide caem num nome genérico.
    assert_eq!(mide::icons::generic_name("mide-files"), Some("system-file-manager"));
    assert_eq!(mide::icons::generic_name("firefox"), None);
    let _ = fs::remove_dir_all(&dir);
}
