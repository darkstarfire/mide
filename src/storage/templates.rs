//! Modelos de "Novo arquivo" (área de trabalho e Arquivos): documentos vazios mas
//! válidos de cada tipo, para o app que abre o tipo conseguir ler (um `.docx` de zero
//! bytes não é um documento do Word; o Editor e o LibreOffice recusam).
//!
//! O nome do arquivo novo é o primeiro livre ("Novo documento.docx", "Novo
//! documento 2.docx"...), na pasta pedida.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Tipos que o "Novo arquivo" oferece: extensão, nome no menu, nome do arquivo novo e
/// ícone do tema.
pub const KINDS: &[(&str, &str, &str, &str)] = &[
    ("txt", "Texto simples (.txt)", "Novo arquivo", "text-x-generic"),
    ("md", "Markdown (.md)", "Novo documento", "text-x-generic"),
    ("docx", "Documento do Word (.docx)", "Novo documento", "x-office-document"),
    ("odt", "Documento do Writer (.odt)", "Novo documento", "x-office-document"),
    ("rtf", "Texto formatado (.rtf)", "Novo documento", "x-office-document"),
    ("xlsx", "Planilha do Excel (.xlsx)", "Nova planilha", "x-office-spreadsheet"),
    ("ods", "Planilha do Calc (.ods)", "Nova planilha", "x-office-spreadsheet"),
    ("csv", "Tabela CSV (.csv)", "Nova tabela", "x-office-spreadsheet"),
    ("json", "JSON (.json)", "Novo arquivo", "text-x-script"),
    ("html", "Página HTML (.html)", "Nova página", "text-html"),
];

/// Conteúdo do documento vazio de um tipo (pela extensão, sem o ponto).
pub fn content(ext: &str) -> Option<Vec<u8>> {
    let empty = crate::richtext::Doc::new();
    Some(match ext {
        "txt" | "csv" => Vec::new(),
        "md" => b"# \n".to_vec(),
        "json" => b"{}\n".to_vec(),
        "html" => b"<!doctype html>\n<html lang=\"pt-BR\">\n<head>\n  <meta charset=\"utf-8\">\n  <title></title>\n</head>\n<body>\n\n</body>\n</html>\n".to_vec(),
        "docx" => crate::richtext::docx::write(&empty),
        "odt" => crate::richtext::odt::write(&empty),
        "rtf" => crate::richtext::rtf::write(&empty).into_bytes(),
        "xlsx" => xlsx()?,
        "ods" => ods()?,
        _ => return None,
    })
}

/// Cria o documento vazio do tipo `ext` na pasta `dir`, com o primeiro nome livre.
pub fn create(dir: &Path, ext: &str) -> std::io::Result<PathBuf> {
    let (_, _, stem, _) = KINDS.iter().find(|(e, ..)| *e == ext).ok_or_else(|| std::io::Error::other("tipo desconhecido"))?;
    let data = content(ext).ok_or_else(|| std::io::Error::other("tipo desconhecido"))?;
    let name = crate::files::numbered_name(dir, stem, ext);
    let path = dir.join(name);
    // `create_new`: nunca escreve por cima de um arquivo que apareceu no meio tempo.
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
    file.write_all(&data)?;
    Ok(path)
}

/// Conteúdo de um documento vazio que o app pode usar quando abre um arquivo de zero
/// bytes desse tipo (criado por outro programa com "novo arquivo vazio").
pub fn is_empty_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.len() == 0)
}

fn zip_of(files: &[(&str, &str, bool)]) -> Option<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        for (name, body, stored) in files {
            let method = if *stored { zip::CompressionMethod::Stored } else { zip::CompressionMethod::Deflated };
            let opts = zip::write::SimpleFileOptions::default().compression_method(method);
            zip.start_file(*name, opts).ok()?;
            zip.write_all(body.as_bytes()).ok()?;
        }
        zip.finish().ok()?;
    }
    Some(out.into_inner())
}

/// Pasta de trabalho do Excel com uma planilha vazia ("Planilha1").
fn xlsx() -> Option<Vec<u8>> {
    zip_of(&[
        (
            "[Content_Types].xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
            false,
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
            false,
        ),
        (
            "xl/workbook.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Planilha1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
            false,
        ),
        (
            "xl/_rels/workbook.xml.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
            false,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData/></worksheet>"#,
            false,
        ),
    ])
}

/// Planilha do LibreOffice Calc com uma tabela vazia. O `mimetype` vem primeiro e sem
/// compressão, como o formato exige.
fn ods() -> Option<Vec<u8>> {
    zip_of(&[
        ("mimetype", "application/vnd.oasis.opendocument.spreadsheet", true),
        (
            "META-INF/manifest.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#,
            false,
        ),
        (
            "content.xml",
            r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2"><office:body><office:spreadsheet><table:table table:name="Planilha1"><table:table-column/><table:table-row><table:table-cell/></table:table-row></table:table></office:spreadsheet></office:body></office:document-content>"#,
            false,
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documentos_novos_sao_validos() {
        let dir = std::env::temp_dir().join(format!("mide-modelos-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (ext, ..) in KINDS {
            let path = create(&dir, ext).unwrap();
            assert!(path.extension().is_some_and(|e| e == *ext), "{ext}");
        }
        // Os formatados voltam a ser lidos pelo próprio leitor do mide.
        let read = |name: &str| std::fs::read(dir.join(name)).unwrap();
        assert!(crate::richtext::docx::read(&read("Novo documento.docx")).is_some());
        assert!(crate::richtext::odt::read(&read("Novo documento.odt")).is_some());
        assert!(crate::richtext::rtf::read(&read("Novo documento.rtf")).is_some());
        // Planilhas: zip com as partes certas.
        let xlsx = zip::ZipArchive::new(std::io::Cursor::new(read("Nova planilha.xlsx"))).unwrap();
        assert!(xlsx.file_names().any(|n| n == "xl/workbook.xml"));
        let mut ods = zip::ZipArchive::new(std::io::Cursor::new(read("Nova planilha.ods"))).unwrap();
        assert_eq!(ods.by_index(0).unwrap().name(), "mimetype");
        // Segundo do mesmo tipo: nome com número, sem apagar o primeiro.
        let again = create(&dir, "docx").unwrap();
        assert_ne!(again, dir.join("Novo documento.docx"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
