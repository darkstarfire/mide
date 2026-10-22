//! Motor da Calculadora do mide (E3b), escrito do zero: contas com precedência e
//! parênteses (modos Básico e Científico), o modo Programador (inteiros em várias
//! bases e operações de bits), a pilha RPN, a memória, o histórico, as conversões de
//! unidades e as notas matemáticas (contas digitadas em texto). Sem interface: o app
//! `mide-calculator` desenha as teclas e chama daqui.
//!
//! O comportamento segue o de uma calculadora de mesa como a do Mac (só como
//! referência de uso, nada copiado): o número digitado aparece na tela, as operações
//! respeitam a precedência (2 + 3 × 4 = 14), `=` repetido refaz a última operação e
//! as funções de uma tecla (x², √, sen...) valem na hora para o número da tela.

pub mod engine;
pub mod format;
pub mod notes;
pub mod programmer;
pub mod rpn;
pub mod units;
