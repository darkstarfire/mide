//! Rotação automática da tela interna pelo sensor de orientação (`iio-sensor-proxy`,
//! o mesmo serviço que o GNOME usa).
//!
//! Como no GNOME: em aparelho com a chave de modo tablet, a tela só gira no modo tablet
//! (no modo notebook fica na posição normal); sem a chave, gira sempre. A trava de
//! rotação (`[display] rotation_lock`, na central de controle e em Configurações →
//! Monitores) deixa a tela como está.
//!
//! A saída ganha a transformação do Wayland (`_90`, `_180`, `_270`): o tamanho lógico
//! troca de lado, as camadas e as janelas se reorganizam (`commit`), e a cena desenha a
//! imagem girada (`render.rs`, por uma textura no tamanho da tela girada).
//!
//! O giro é animado, como no iPad: o quadro de antes vira uma foto, que gira e esmaece
//! enquanto a cena nova entra girando do ângulo antigo até o novo, as duas sempre
//! cabendo na tela. Por isso o giro pedido (`set`) só vale no próximo quadro da tela,
//! depois da foto (`take_pending` + `commit`, chamados pela cena).
//!
//! Aparelho deitado (o sensor diz `undefined`): fica a última orientação válida, como
//! no iPad. Ao entrar no modo tablet, a orientação é lida na hora (`read_now`).

use std::time::{Duration, Instant};

use smithay::{
    desktop::{layer_map_for_output, Window},
    output::Output,
    utils::{Logical, Rectangle, Transform},
};

use crate::state::State;

/// Giro da tela (`mide::motion`, a mesma tabela das outras animações).
pub const DURATION: Duration = mide::motion::SCREEN_ROTATE;

/// Duração do giro; `MIDE_ROTATE_MS` muda (os testes salvam quadros devagar e precisam
/// de um giro longo para ver o meio dele).
fn duration() -> Duration {
    static MS: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    MS.get_or_init(|| std::env::var("MIDE_ROTATE_MS").ok().and_then(|v| v.parse().ok())).map_or(DURATION, Duration::from_millis)
}

/// Sem quadro (tela apagada), o giro pedido vale depois disto, sem animação.
const PENDING_LIMIT: Duration = Duration::from_millis(300);

const SENSOR_NAME: &str = "net.hadess.SensorProxy";
const SENSOR_PATH: &str = "/net/hadess/SensorProxy";

/// Giro pedido e animação em andamento.
#[derive(Debug, Default)]
pub struct Rotation {
    /// Giro ainda não aplicado: o próximo quadro da tela tira a foto da cena como está e
    /// só então gira (a foto é a imagem antiga da animação).
    pub pending: Option<Pending>,
    /// Animação em andamento (a foto fica no `render.rs`).
    pub anim: Option<Anim>,
    /// Canal do sensor (para a leitura na hora, ao entrar no modo tablet).
    pub sender: Option<smithay::reexports::calloop::channel::Sender<SensorEvent>>,
}

#[derive(Debug, Clone)]
pub struct Pending {
    pub output: String,
    pub to: Transform,
    pub since: Instant,
}

#[derive(Debug, Clone)]
pub struct Anim {
    pub output: String,
    pub from: Transform,
    pub to: Transform,
    pub start: Instant,
}

impl Anim {
    /// Progresso da animação (0 a 1), já com a curva.
    pub fn eased(&self) -> f32 {
        mide::motion::ease_in_out(mide::motion::progress(self.start, duration()))
    }

    pub fn done(&self) -> bool {
        self.start.elapsed() >= duration()
    }
}

/// Transformação da tela para a orientação lida do sensor (a mesma do mutter).
pub fn transform_for(orientation: &str) -> Transform {
    match orientation {
        "left-up" => Transform::_90,
        "right-up" => Transform::_270,
        "bottom-up" => Transform::_180,
        _ => Transform::Normal,
    }
}

/// Ângulo (radianos; positivo é o sentido horário na tela, com o y para baixo) em que a
/// cena da tela girada aparece no painel. Conferido com o desenho do Smithay: em `_90`
/// o topo da cena fica na borda direita do painel (giro no sentido horário,
/// como o `TRANSFORM_90` do mutter, que é o que ele usa para `left-up`).
pub fn angle_of(transform: Transform) -> f32 {
    use std::f32::consts::{FRAC_PI_2, PI};
    match transform {
        Transform::_90 | Transform::Flipped90 => FRAC_PI_2,
        Transform::_180 | Transform::Flipped180 => PI,
        Transform::_270 | Transform::Flipped270 => -FRAC_PI_2,
        _ => 0.0,
    }
}

/// Diferença de ângulo de `from` para `to` pelo caminho mais curto (−π a π).
pub fn turn(from: Transform, to: Transform) -> f32 {
    use std::f32::consts::{PI, TAU};
    let mut d = angle_of(to) - angle_of(from);
    while d > PI + 1e-4 {
        d -= TAU;
    }
    while d < -PI - 1e-4 {
        d += TAU;
    }
    d
}

/// A maior escala em que um retângulo `size` girado de `angle` cabe inteiro em `screen`.
pub fn fit_zoom(size: (f32, f32), angle: f32, screen: (f32, f32)) -> f32 {
    let (c, s) = (angle.cos().abs(), angle.sin().abs());
    let bw = size.0 * c + size.1 * s;
    let bh = size.0 * s + size.1 * c;
    (screen.0 / bw.max(1.0)).min(screen.1 / bh.max(1.0))
}

/// A tela gira com o sensor agora? Com a chave de modo tablet, só no modo tablet.
pub fn auto_rotate(state: &State) -> bool {
    state.touch.sensor && (state.touch.tablet || !state.touch.tablet_switch)
}

/// Põe a tela interna na orientação que vale agora (sensor, modo tablet e trava).
pub fn apply(state: &mut State) {
    if state.config.display.rotation_lock {
        return;
    }
    let wanted = if auto_rotate(state) {
        state.touch.orientation.as_deref().map(transform_for).unwrap_or(Transform::Normal)
    } else {
        Transform::Normal
    };
    set(state, wanted);
}

/// Pede o giro da tela interna para `transform`. Com as animações ligadas, o giro
/// acontece no próximo quadro dela (depois da foto da cena antiga); sem elas, já.
pub fn set(state: &mut State, transform: Transform) {
    let Some(output) = crate::touch::internal_output(state) else {
        return;
    };
    let target = state.rotation.pending.as_ref().filter(|p| p.output == output.name()).map(|p| p.to).unwrap_or_else(|| output.current_transform());
    if target == transform {
        return;
    }
    // Volta à orientação atual antes de o giro anterior acontecer: nada a fazer.
    if output.current_transform() == transform {
        state.rotation.pending = None;
        return;
    }
    if !state.config.effects.animations {
        commit(state, &output, transform);
        return;
    }
    tracing::info!("rotação: pedido de {transform:?} na tela {}", output.name());
    state.rotation.pending = Some(Pending { output: output.name(), to: transform, since: Instant::now() });
    // Sem quadro a tempo (tela apagada, sessão em outro console), gira sem a animação.
    let timer = smithay::reexports::calloop::timer::Timer::from_duration(PENDING_LIMIT + Duration::from_millis(20));
    let _ = state.loop_handle.insert_source(timer, |_, _, state: &mut State| {
        commit_overdue(state);
        smithay::reexports::calloop::timer::TimeoutAction::Drop
    });
    state.wake();
}

/// Giro pedido que nenhum quadro atendeu: vale agora, sem animação.
fn commit_overdue(state: &mut State) {
    let Some(pending) = state.rotation.pending.clone().filter(|p| p.since.elapsed() >= PENDING_LIMIT) else {
        return;
    };
    state.rotation.pending = None;
    let output = state.space.outputs().find(|o| o.name() == pending.output).cloned();
    if let Some(output) = output {
        tracing::info!("rotação: sem quadro a tempo, girando sem animação");
        commit(state, &output, pending.to);
    }
}

/// Giro pedido para esta tela (o quadro tira a foto e chama `commit`).
pub fn take_pending(state: &mut State, output: &Output) -> Option<Transform> {
    if state.rotation.pending.as_ref().is_some_and(|p| p.output == output.name()) {
        return state.rotation.pending.take().map(|p| p.to);
    }
    None
}

/// Gira a tela para `transform` e refaz tudo para o tamanho novo: camadas (barra, dock,
/// mesa, launcher, teclado) reconfiguradas pelo arranjo, janelas maximizadas, encaixadas
/// e em tela cheia redimensionadas, janelas soltas trazidas para dentro da tela, e a
/// escala de cada superfície mandada de novo (nítidas depois do giro).
pub fn commit(state: &mut State, output: &Output, transform: Transform) {
    let name = output.name();
    let from = output.current_transform();
    if from == transform {
        return;
    }
    tracing::info!("rotação: tela {name} em {transform:?}");
    // Como cada janela está na tela antes do giro (maximizada, numa parte, solta).
    let before = match (state.space.output_geometry(output), state.usable_area_of(output)) {
        (Some(geo), Some(area)) => Some((crate::outputs::windows_on(state, &name, geo, area), geo, area)),
        _ => None,
    };
    output.change_current_state(None, Some(transform), None, None);
    let live: Vec<Output> = state.space.outputs().cloned().collect();
    crate::outputs::place(state, &live);
    layer_map_for_output(output).arrange();
    if let (Some((windows, geo, old_area)), Some(area)) = (before, state.usable_area_of(output)) {
        relayout(state, windows, geo, old_area, area);
    }
    crate::activation::update_scales(state, output);
    if state.config.effects.animations {
        state.rotation.anim = Some(Anim { output: name, from, to: transform, start: Instant::now() });
    }
    state.wake();
}

/// Janelas da tela que girou, no lugar equivalente da tela nova.
fn relayout(state: &mut State, windows: Vec<crate::outputs::Displaced<Window>>, old_geo: Rectangle<i32, Logical>, old_area: Rectangle<i32, Logical>, area: Rectangle<i32, Logical>) {
    use crate::outputs::{from_fraction, Fit};
    let mut moved = Vec::new();
    for d in windows {
        let window = d.window;
        if state.fit_fullscreen(&window) {
            continue;
        }
        let Some(toplevel) = window.toplevel().cloned() else {
            continue;
        };
        moved.push(toplevel.wl_surface().clone());
        // Modo tablet com os apps em tela inteira: toda janela que dá ocupa a área.
        let fit = if crate::touch::maximize_windows(state) && !crate::state::fixed_size(&window) { Fit::Maximized } else { d.fit };
        match fit {
            Fit::Maximized => state.maximize_in(&toplevel, area),
            Fit::Part { frac, left } => {
                let r = from_fraction(frac, (area.loc.x, area.loc.y, area.size.w, area.size.h));
                state.snap_to(&toplevel, Rectangle::new((r.0, r.1).into(), (r.2, r.3).into()), left);
            }
            Fit::Free => {
                let size = window.geometry().size;
                let bar = crate::state::bar_height_for(&window);
                let (w, h) = free_size((size.w, size.h), (area.size.w, (area.size.h - bar).max(1)));
                if (w, h) != (size.w, size.h) && !crate::state::fixed_size(&window) {
                    toplevel.with_pending_state(|s| s.size = Some((w, h).into()));
                    toplevel.send_configure();
                }
                let rel = (d.rel.0 + old_geo.loc.x - old_area.loc.x, d.rel.1 + old_geo.loc.y - old_area.loc.y - bar);
                let (x, y) = free_place(rel, (old_area.size.w, old_area.size.h), (w, h + bar), (area.size.w, area.size.h));
                state.space.map_element(window.clone(), (area.loc.x + x, area.loc.y + y + bar), false);
            }
        }
    }
    // A animação do giro já mostra a mudança: as janelas vão direto ao lugar novo.
    state.animations.retain(|a| !moved.contains(&a.surface));
}

/// Tamanho de uma janela solta na tela girada: encolhe só o que passa da área útil.
fn free_size(size: (i32, i32), max: (i32, i32)) -> (i32, i32) {
    (size.0.min(max.0).max(1), size.1.min(max.1).max(1))
}

/// Canto de uma janela solta (com a barra) na área nova: a mesma fração da área antiga,
/// sem sair dela.
fn free_place(rel: (i32, i32), old: (i32, i32), size: (i32, i32), area: (i32, i32)) -> (i32, i32) {
    let fx = rel.0 as f32 / old.0.max(1) as f32;
    let fy = rel.1 as f32 / old.1.max(1) as f32;
    (((fx * area.0 as f32) as i32).clamp(0, (area.0 - size.0).max(0)), ((fy * area.1 as f32) as i32).clamp(0, (area.1 - size.1).max(0)))
}

/// A tela é desenhada por uma textura intermediária (girada, girando ou para girar):
/// sem cursor de hardware nem scanout direto.
pub fn via_texture(state: &State, output: &Output) -> bool {
    rotated(output)
        || state.rotation.anim.as_ref().is_some_and(|a| a.output == output.name())
        || state.rotation.pending.as_ref().is_some_and(|p| p.output == output.name())
}

/// A tela está girada.
pub fn rotated(output: &Output) -> bool {
    !matches!(output.current_transform(), Transform::Normal | Transform::Flipped180)
}

/// O que o sensor manda ao laço principal.
#[derive(Debug)]
pub enum SensorEvent {
    /// Há (ou não) acelerômetro.
    Present(bool),
    Orientation(String),
}

/// Lê o `iio-sensor-proxy` numa thread (barramento do sistema) e manda a orientação
/// ao laço principal. Sem o serviço ou sem acelerômetro, não faz nada.
pub fn start_sensor(state: &mut State) {
    let (sender, channel) = smithay::reexports::calloop::channel::channel::<SensorEvent>();
    let inserted = state.loop_handle.insert_source(channel, |event, _, state: &mut State| {
        let smithay::reexports::calloop::channel::Event::Msg(event) = event else {
            return;
        };
        match event {
            SensorEvent::Present(present) => {
                tracing::info!("sensor de orientação: {}", if present { "presente" } else { "ausente" });
                state.touch.sensor = present;
            }
            SensorEvent::Orientation(orientation) => orientation_changed(state, orientation),
        }
    });
    if let Err(err) = inserted {
        tracing::warn!("sensor de orientação sem canal: {err}");
        return;
    }
    state.rotation.sender = Some(sender.clone());
    std::thread::Builder::new()
        .name("mide-sensor".into())
        .spawn(move || {
            if let Err(err) = watch(&sender) {
                tracing::info!("sensor de orientação indisponível: {err}");
            }
        })
        .ok();
}

/// Orientação nova do sensor (ou da leitura na hora). Deitado (`undefined`), fica a
/// última válida.
pub fn orientation_changed(state: &mut State, orientation: String) {
    if !valid(&orientation) {
        tracing::info!("sensor de orientação: {orientation} (fica a última: {})", state.touch.orientation.as_deref().unwrap_or("normal"));
        return;
    }
    if state.touch.orientation.as_deref() != Some(orientation.as_str()) {
        tracing::info!("sensor de orientação: {orientation}");
    }
    state.touch.orientation = Some(orientation);
    apply(state);
}

/// Orientação que serve para girar a tela (deitado não serve).
fn valid(orientation: &str) -> bool {
    matches!(orientation, "normal" | "left-up" | "right-up" | "bottom-up")
}

/// Lê a orientação do sensor agora (ao entrar no modo tablet o aparelho pode já estar
/// em pé, sem mudança nenhuma a avisar). Numa thread, como a leitura contínua.
pub fn read_now(state: &State) {
    let Some(sender) = state.rotation.sender.clone() else {
        return;
    };
    std::thread::Builder::new()
        .name("mide-sensor-agora".into())
        .spawn(move || {
            let Ok(conn) = zbus::blocking::Connection::system() else {
                return;
            };
            if let Ok(orientation) = sensor_proxy(&conn).and_then(|s| s.get_property::<String>("AccelerometerOrientation")) {
                let _ = sender.send(SensorEvent::Orientation(orientation));
            }
        })
        .ok();
}

fn sensor_proxy(conn: &zbus::blocking::Connection) -> zbus::Result<zbus::blocking::Proxy<'static>> {
    zbus::blocking::proxy::Builder::new(conn)
        .destination(SENSOR_NAME)?
        .path(SENSOR_PATH)?
        .interface(SENSOR_NAME)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
}

fn watch(sender: &smithay::reexports::calloop::channel::Sender<SensorEvent>) -> zbus::Result<()> {
    let conn = zbus::blocking::Connection::system()?;
    let sensor = sensor_proxy(&conn)?;
    let present: bool = sensor.get_property("HasAccelerometer").unwrap_or(false);
    let _ = sender.send(SensorEvent::Present(present));
    if !present {
        return Ok(());
    }
    sensor.call::<_, _, ()>("ClaimAccelerometer", &())?;
    let mut last = String::new();
    let mut send = |sensor: &zbus::blocking::Proxy| {
        if let Ok(orientation) = sensor.get_property::<String>("AccelerometerOrientation")
            && orientation != last
        {
            last = orientation.clone();
            let _ = sender.send(SensorEvent::Orientation(orientation));
        }
    };
    send(&sensor);
    let properties = zbus::blocking::Proxy::new(&conn, SENSOR_NAME, SENSOR_PATH, "org.freedesktop.DBus.Properties")?;
    for _ in properties.receive_signal("PropertiesChanged")? {
        send(&sensor);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientacao_do_sensor_para_a_tela() {
        assert_eq!(transform_for("normal"), Transform::Normal);
        assert_eq!(transform_for("left-up"), Transform::_90);
        assert_eq!(transform_for("right-up"), Transform::_270);
        assert_eq!(transform_for("bottom-up"), Transform::_180);
        assert_eq!(transform_for("undefined"), Transform::Normal);
        // Deitado não gira nada (fica a última).
        assert!(!valid("undefined"));
        assert!(valid("left-up"));
    }

    #[test]
    fn giro_pelo_caminho_mais_curto() {
        use std::f32::consts::{FRAC_PI_2, PI};
        assert!((turn(Transform::Normal, Transform::_90) - FRAC_PI_2).abs() < 1e-4);
        assert!((turn(Transform::Normal, Transform::_270) + FRAC_PI_2).abs() < 1e-4);
        assert!((turn(Transform::_90, Transform::_270).abs() - PI).abs() < 1e-4);
        // De 270 para normal: um quarto de volta, não três.
        assert!((turn(Transform::_270, Transform::Normal) - FRAC_PI_2).abs() < 1e-4);
    }

    #[test]
    fn foto_girada_cabe_na_tela() {
        use std::f32::consts::FRAC_PI_2;
        // Parada, cabe exata.
        assert!((fit_zoom((1366.0, 768.0), 0.0, (1366.0, 768.0)) - 1.0).abs() < 1e-4);
        // Deitada de lado: encolhe para a altura caber na largura.
        let z = fit_zoom((1366.0, 768.0), FRAC_PI_2, (1366.0, 768.0));
        assert!((z - 768.0 / 1366.0).abs() < 1e-3);
        // A cena em retrato, no ângulo dela, cabe exata.
        assert!((fit_zoom((768.0, 1366.0), FRAC_PI_2, (1366.0, 768.0)) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn janela_solta_na_tela_girada() {
        // 900 de largura numa tela de 768: encolhe só a largura.
        assert_eq!(free_size((900, 500), (768, 1300)), (768, 500));
        // No meio da tela deitada, no meio da tela em pé, sem sair dela.
        assert_eq!(free_place((683, 0), (1366, 700), (400, 300), (768, 1300)), (368, 0));
        assert_eq!(free_place((1300, 600), (1366, 700), (400, 300), (768, 1300)), (368, 1000));
    }
}
