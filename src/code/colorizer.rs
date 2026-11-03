//! Cores do código numa thread à parte, para a digitação nunca esperar o realce: o
//! arquivo inteiro levava de 15 ms (realce próprio, 3 mil linhas) a centenas de
//! milissegundos (gramática do tree-sitter em WebAssembly) a cada tecla. O app pede
//! as cores de uma versão do texto e, enquanto elas não chegam, desloca as que já tem
//! pela edição ([`shift_spans`]), sem piscar.
//!
//! Para quem vem do Go: a thread é uma goroutine com dois canais (`mpsc`), um de
//! pedidos e um de respostas; a thread é dona do `Syntax` (o analisador não sai dela).

use std::{
    ops::Range,
    sync::mpsc::{self, Receiver, Sender},
};

use crate::code::{highlight::Kind, lang::Language as Lang, syntax::Syntax};

/// Cores prontas de uma versão do texto de um documento, com o próprio texto (para o
/// app deslocar as cores até a versão de agora, se já digitaram mais).
pub struct Colored {
    pub doc: u64,
    pub version: u64,
    pub text: String,
    pub spans: Vec<(Range<usize>, Kind)>,
}

enum Job {
    Color { doc: u64, version: u64, text: String, lang: &'static Lang },
    Forget(u64),
}

/// A thread do realce: recebe pedidos e devolve as cores, sempre só do pedido mais
/// novo de cada documento (os de versões que já passaram são pulados).
pub struct Colorizer {
    jobs: Sender<Job>,
    done: Receiver<Colored>,
}

impl Colorizer {
    /// Abre a thread; `wake` acorda a janela quando há cores prontas.
    pub fn new(wake: impl Fn() + Send + 'static) -> Self {
        let (jobs, rx) = mpsc::channel::<Job>();
        let (tx, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("mide-code-colors".into())
            .spawn(move || {
                let mut syntax = Syntax::new();
                while let Ok(first) = rx.recv() {
                    // Junta os pedidos que chegaram enquanto o último era feito: de cada
                    // documento, só o mais novo.
                    let mut pending: Vec<Job> = vec![first];
                    pending.extend(rx.try_iter());
                    let mut latest: Vec<(u64, u64, String, &'static Lang)> = Vec::new();
                    for job in pending {
                        match job {
                            Job::Forget(doc) => {
                                syntax.forget(doc);
                                latest.retain(|j| j.0 != doc);
                            }
                            Job::Color { doc, version, text, lang } => {
                                latest.retain(|j| j.0 != doc);
                                latest.push((doc, version, text, lang));
                            }
                        }
                    }
                    for (doc, version, text, lang) in latest {
                        let spans = syntax.highlight_doc(doc, &text, lang).unwrap_or_else(|| crate::code::highlight::highlight(&text, lang));
                        if tx.send(Colored { doc, version, text, spans }).is_err() {
                            return;
                        }
                        wake();
                    }
                }
            })
            .expect("thread das cores");
        Self { jobs, done }
    }

    /// Pede as cores da versão `version` do texto do documento `doc`.
    pub fn request(&self, doc: u64, version: u64, text: String, lang: &'static Lang) {
        let _ = self.jobs.send(Job::Color { doc, version, text, lang });
    }

    /// O documento fechou: a thread esquece a árvore dele.
    pub fn forget(&self, doc: u64) {
        let _ = self.jobs.send(Job::Forget(doc));
    }

    /// Cores que ficaram prontas desde a última vez.
    pub fn poll(&self) -> Vec<Colored> {
        self.done.try_iter().collect()
    }
}

/// Desloca as cores do texto `old` para valerem no texto `new` (a edição é o trecho
/// entre o começo e o fim em comum): as de antes da edição ficam, as de depois andam
/// junto, a que contém a edição cresce ou encolhe (digitando no meio de uma palavra) e
/// as cortadas por ela perdem a parte apagada. As cores continuam em ordem e sem
/// sobreposição.
pub fn shift_spans<T: Copy>(spans: &mut Vec<(Range<usize>, T)>, old: &str, new: &str) {
    let Some((start, (old_end, new_end))) = crate::code::syntax::edit_range(old, new) else {
        return;
    };
    let moved = |p: usize| -> usize { (p as isize + new_end as isize - old_end as isize) as usize };
    // Letras digitadas logo depois de uma palavra continuam a palavra (e a cor dela).
    let word_typed = old_end == start && new[start..new_end].chars().all(|c| c.is_alphanumeric() || c == '_');
    spans.retain_mut(|(r, _)| {
        if r.end == start && word_typed && r.start < r.end {
            r.end = new_end;
        } else if r.end <= start {
            // Antes da edição (ou terminando nela): fica como está.
        } else if r.start >= old_end && (r.start > start || old_end == start) {
            // Depois da edição: anda junto.
            *r = moved(r.start)..moved(r.end);
        } else if r.start <= start && r.end >= old_end {
            // Contém a edição: cresce ou encolhe com ela.
            r.end = moved(r.end);
        } else if r.start < start {
            // Começa antes e termina dentro do trecho apagado: perde o fim.
            r.end = start;
        } else {
            // Começa dentro do trecho apagado: o que sobra vem depois do texto novo.
            *r = new_end..moved(r.end.max(old_end));
        }
        r.start < r.end && r.end <= new.len()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shifted(spans: &[(Range<usize>, u8)], old: &str, new: &str) -> Vec<(Range<usize>, u8)> {
        let mut s = spans.to_vec();
        shift_spans(&mut s, old, new);
        s
    }

    #[test]
    fn cores_acompanham_a_digitacao() {
        // "let a = 1;" com `let` (1), `a` (2) e `1` (3).
        let old = "let a = 1;";
        let spans = [(0..3, 1), (4..5, 2), (8..9, 3)];
        // Digitar no fim da palavra `a` ("ab"): ela cresce e o `1` anda.
        assert_eq!(shifted(&spans, old, "let ab = 1;"), vec![(0..3, 1), (4..6, 2), (9..10, 3)]);
        // Uma linha nova no começo: tudo anda.
        assert_eq!(shifted(&spans, old, "\nlet a = 1;"), vec![(1..4, 1), (5..6, 2), (9..10, 3)]);
        // Apagar o `a`: a cor dele some, as outras voltam.
        assert_eq!(shifted(&spans, old, "let  = 1;"), vec![(0..3, 1), (7..8, 3)]);
        // Apagar do meio de `let` até o `=`: `let` perde o fim, o `1` volta.
        assert_eq!(shifted(&spans, old, "le= 1;"), vec![(0..2, 1), (4..5, 3)]);
        // Texto igual: nada muda.
        assert_eq!(shifted(&spans, old, old), spans.to_vec());
    }

    #[test]
    fn cores_com_acentos() {
        // Digitar "é" (2 bytes) dentro da string.
        let old = "s = \"ca\";";
        let spans = [(4..8, 1)];
        assert_eq!(shifted(&spans, old, "s = \"caé\";"), vec![(4..10, 1)]);
    }

    #[test]
    fn thread_devolve_so_a_versao_mais_nova() {
        let lang = crate::code::lang::detect(std::path::Path::new("a.rs"), "");
        let (wake_tx, wake_rx) = mpsc::channel();
        let c = Colorizer::new(move || {
            let _ = wake_tx.send(());
        });
        for v in 1..=5u64 {
            c.request(7, v, format!("fn f{v}() {{}}"), lang);
        }
        let mut last = None;
        while last.is_none_or(|v| v != 5) {
            wake_rx.recv_timeout(std::time::Duration::from_secs(5)).expect("cores prontas");
            for done in c.poll() {
                assert_eq!(done.doc, 7);
                assert!(!done.spans.is_empty());
                last = Some(done.version);
            }
        }
    }
}
