//! Sistema e sessão: estado e ações do sistema (som, rede, energia), informações da máquina, telas, teclado, aparência, configuração, itens de início, senha de administrador, portal e canal de controle do compositor.

pub mod system;
pub mod sysinfo;
pub mod displays;
pub mod keyboard_layouts;
pub mod sounds;
pub mod appearance;
pub mod settings_file;
pub mod login_items;
pub mod elevate;
pub mod portal;
pub mod control_client;
