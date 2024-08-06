use crate::{Document, TfIdf};
use std::collections::HashMap;
use crate::intern::{intern, get_str};

impl TfIdf {
    // TODO: Could significantly reduce file size (and ram size if done on a
    // structural level) by having document title and terms be an index into
    // the global term. This essential "interns" the terms, and the program
    // can maintain a global pool of terms.
    //
    // Terms are repeated twice and titles are repeated twice
    pub fn deserialize(b: &[u8]) -> Result<Self, String> {
        let mut tf_idf = Self::default();
        let mut document: Option<Document> = None;
        let mut i = 0;
        while i < b.len() {
            let mut offset;
            let c = b[i];
            // 0x01 global term    => 01 {term len}x2 {count}x4
            // 0x02 document title => 02 {title len}x2
            // 0x03 document path  => 03 {path len}x2
            // 0x04 document term  => 04 {term len}x2 {count}x4
            match c {
                0x01 => {
                    let term_len = u16::from_le_bytes(b[i + 1..][..2].try_into().unwrap());
                    let count = u64::from_le_bytes(b[i + 3..][..8].try_into().unwrap());
                    offset = 1 + 2 + 8;
                    let term = String::from_utf8(b[i + offset..][..term_len as usize].to_vec())
                        .expect("This should be valid utf8");
                    let id = intern(term);
                    tf_idf.global_term_count.insert(id, count as usize);

                    offset = 1 + 2 + 8 + term_len as usize;
                }
                0x02 => {
                    if let Some(doc) = document.take() {
                        tf_idf.documents.insert(doc.title.clone(), doc);
                    }
                    let title_len = u16::from_le_bytes(b[i + 1..][..2].try_into().unwrap());
                    offset = 1 + 2;
                    let title = String::from_utf8(b[i + offset..][..title_len as usize].to_vec())
                        .expect("This should be valid utf8");
                    document = Some(Document {
                        path: String::new(),
                        title,
                        term_frequency: HashMap::new(),
                    });
                    offset = 1 + 2 + title_len as usize;
                }
                0x03 => {
                    let doc = match document.as_mut() {
                        Some(doc) => doc,
                        None => {
                            return Err(format!(
                                "Bytes not in correct order; potentially corrupted cache file"
                            ))
                        }
                    };
                    let path_len = u16::from_le_bytes(b[i + 1..][..2].try_into().unwrap());
                    offset = 1 + 2;
                    let path = String::from_utf8(b[i + offset..][..path_len as usize].to_vec())
                        .expect("This should be valid utf8");
                    doc.path = path;
                    offset = 1 + 2 + path_len as usize;
                }
                0x04 => {
                    let doc = match document.as_mut() {
                        Some(doc) => doc,
                        None => {
                            return Err(format!(
                                "Bytes not in correct order; potentially corrupted cache file"
                            ))
                        }
                    };
                    let term_len = u16::from_le_bytes(b[i + 1..][..2].try_into().unwrap());
                    let count = f64::from_le_bytes(b[i + 3..][..8].try_into().unwrap());
                    offset = 1 + 2 + 8;
                    let term = String::from_utf8(b[i + offset..][..term_len as usize].to_vec())
                        .expect("This should be valid utf8");
                    let id = intern(term);
                    doc.term_frequency.insert(id, count);

                    offset = 1 + 2 + 8 + term_len as usize;
                }
                _ => {
                    dbg!(tf_idf);
                    dbg!(document);
                    return Err(format!(
                        "Unknown mode byte; potentially corrupted cache file: {c} at idx {i}"
                    ));
                }
            }
            i += offset;
        }
        Ok(tf_idf)
    }

    pub fn serialize(&self, writer: &mut impl std::io::Write) -> Result<(), std::io::Error> {
        for (term, count) in &self.global_term_count {
            writer.write(&[0x01])?;
            writer.write(&(get_str(*term).len() as u16).to_le_bytes())?;
            writer.write(&(*count as u64).to_le_bytes())?;
            write!(writer, "{}", term)?;
        }
        for (_, doc) in &self.documents {
            writer.write(&[0x02])?;
            writer.write(&(doc.title.len() as u16).to_le_bytes())?;
            write!(writer, "{}", doc.title)?;
            writer.write(&[0x03])?;
            writer.write(&(doc.path.len() as u16).to_le_bytes())?;
            write!(writer, "{}", doc.path)?;
            for (term, freq) in &doc.term_frequency {
                writer.write(&[0x04])?;
                writer.write(&(get_str(*term).len() as u16).to_le_bytes())?;
                writer.write(&(*freq).to_le_bytes())?;
                write!(writer, "{}", term)?;
            }
        }
        Ok(())
    }
}

// TODO: Write some tests
