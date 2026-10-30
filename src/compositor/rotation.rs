//! Rotação automática da tela interna pelo sensor de orientação (`iio-sensor-proxy`,
//! o mesmo serviço que o GNOME usa).
//!
//! Como no GNOME: em aparelho com a chave de modo tablet, a tela só gira no modo tablet
//! (no modo notebook fica na posição normal); sem a chave, gira sempre. A trava de
//! rotação (`[display] rotation_lock`, na central de controle e em Configurações →
//! Monitores) deixa a tela como está.
//!
//! A saída ganha a transformação do Wayland (`_90`, `_180`, `_270`): o tamanho lógico
//! troca de lado, as camadas e as janelas se reorganizam, e a cena desenha a imagem
//! girada (`render.rs`, por uma textura no tamanho da tela girada).

use std::time::{Duration, Instant};

use smithay::{desktop::layer_map_for_output, utils::Transform};

use crate::state::State;

/// Esmaecer da troca de orientação.
pub const FADE: Duration = Duration::from_millis(260);

/// Transformação da tela para a orientação lida do sensor (a mesma do mutter).
pub fn transform_for(orientation: &str) -> Transform {
    match orientation {
        "left-up" => Transform::_90,
        "right-up" => Transform::_270,
        "bottom-up" => Transform::_180,
        _ => Transform::Normal,
    }
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

/// Gira a tela interna para `transform` e reorganiza camadas e janelas.
pub fn set(state: &mut State, transform: Transform) {
    let Some(output) = crate::touch::internal_output(state) else {
        return;
    };
    if output.current_transform() == transform {
        return;
    }
    tracing::info!("rotação: tela {} em {transform:?}", output.name());
    output.change_current_state(None, Some(transform), None, None);
    let live: Vec<smithay::output::Output> = state.space.outputs().cloned().collect();
    crate::outputs::place(state, &live);
    layer_map_for_output(&output).arrange();
    state.fit_windows_to_area();
    if state.config.effects.animations {
        state.rotation_fade = Some(Instant::now());
    }
    state.wake();
}

/// Opacidade do véu preto da troca de orientação (1 logo depois, 0 no fim).
pub fn fade(state: &State) -> Option<f32> {
    let since = state.rotation_fade?;
    let t = since.elapsed().as_secs_f32() / FADE.as_secs_f32();
    (t < 1.0).then_some(1.0 - t * t * (3.0 - 2.0 * t))
}

/// A tela está girada (a cena desenha pela textura intermediária; sem cursor de
/// hardware nem scanout direto).
pub fn rotated(output: &smithay::output::Output) -> bool {
    !matches!(output.current_transform(), Transform::Normal | Transform::Flipped180)
}

/// O que o sensor manda ao laço principal.
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
            SensorEvent::Orientation(orientation) => {
                tracing::info!("sensor de orientação: {orientation}");
                state.touch.orientation = Some(orientation);
                apply(state);
            }
        }
    });
    if let Err(err) = inserted {
        tracing::warn!("sensor de orientação sem canal: {err}");
        return;
    }
    std::thread::Builder::new()
        .name("mide-sensor".into())
        .spawn(move || {
            if let Err(err) = watch(&sender) {
                tracing::info!("sensor de orientação indisponível: {err}");
            }
        })
        .ok();
}

fn watch(sender: &smithay::reexports::calloop::channel::Sender<SensorEvent>) -> zbus::Result<()> {
    const NAME: &str = "net.hadess.SensorProxy";
    const PATH: &str = "/net/hadess/SensorProxy";
    let conn = zbus::blocking::Connection::system()?;
    let sensor: zbus::blocking::Proxy = zbus::blocking::proxy::Builder::new(&conn)
        .destination(NAME)?
        .path(PATH)?
        .interface(NAME)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()?;
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
            && orientation != "undefined"
        {
            last = orientation.clone();
            let _ = sender.send(SensorEvent::Orientation(orientation));
        }
    };
    send(&sensor);
    let properties = zbus::blocking::Proxy::new(&conn, NAME, PATH, "org.freedesktop.DBus.Properties")?;
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
    }
}
