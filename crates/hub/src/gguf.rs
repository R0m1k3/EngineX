//! Lecture des métadonnées d'un fichier GGUF (en-tête et paires clé/valeur),
//! en flux : les poids ne sont jamais lus. Les fichiers viennent d'Internet :
//! toutes les longueurs sont bornées.

use std::{
    collections::{BTreeMap, HashMap},
    io::{self, Read},
    path::Path,
};

use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 4] = b"GGUF";
const MAX_STRING: u64 = 64 << 20;
const MAX_KV: u64 = 1 << 20;
const MAX_ARRAY: u64 = 1 << 27;
/// Tableaux d'entiers gardés en mémoire (têtes KV par couche...) ; au-delà,
/// seule la longueur est retenue.
const KEEP_ARRAY: u64 = 4096;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum GgufError {
    #[error("pas un fichier GGUF")]
    NotGguf,
    /// Le flux s'arrête avant la fin des métadonnées : il faut plus d'octets.
    #[error("métadonnées tronquées")]
    Truncated,
    #[error("valeur trop grande : {0}")]
    TooLarge(&'static str),
    #[error("{0}")]
    Io(String),
}

impl From<io::Error> for GgufError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::UnexpectedEof {
            Self::Truncated
        } else {
            Self::Io(e.to_string())
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GgufMeta {
    pub arch: String,
    pub name: Option<String>,
    pub size_label: Option<String>,
    pub file_type: Option<u32>,
    pub context_length: Option<u64>,
    pub block_count: Option<u64>,
    pub embedding_length: Option<u64>,
    pub head_count: Option<u64>,
    pub head_count_kv: Option<u64>,
    pub key_length: Option<u64>,
    pub value_length: Option<u64>,
    pub sliding_window: Option<u64>,
    pub vocab_size: Option<u64>,
    pub has_chat_template: bool,
    pub split_count: Option<u64>,
    /// Clés `general.sampling.*` (suffixe → valeur).
    pub sampling: BTreeMap<String, f64>,
}

#[derive(Clone, Debug)]
enum Value {
    Int(i128),
    Float(f64),
    Bool(bool),
    Str(String),
    /// Tableau : longueur, et les entiers s'il est court.
    Array {
        len: u64,
        ints: Option<Vec<i128>>,
    },
}

impl Value {
    fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Int(i) => u64::try_from(*i).ok(),
            Self::Array { ints: Some(v), .. } => v.iter().copied().max().and_then(|m| u64::try_from(m).ok()),
            _ => None,
        }
    }
    fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Int(i) => Some(*i as f64),
            Self::Float(f) => Some(*f),
            _ => None,
        }
    }
}

struct Reader<R> {
    r: R,
}

impl<R: Read> Reader<R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], GgufError> {
        let mut b = [0u8; N];
        self.r.read_exact(&mut b)?;
        Ok(b)
    }
    fn u32(&mut self) -> Result<u32, GgufError> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    fn u64(&mut self) -> Result<u64, GgufError> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }
    fn skip(&mut self, n: u64) -> Result<(), GgufError> {
        let copied = io::copy(&mut (&mut self.r).take(n), &mut io::sink())?;
        if copied < n {
            return Err(GgufError::Truncated);
        }
        Ok(())
    }
    fn string_len(&mut self) -> Result<u64, GgufError> {
        let n = self.u64()?;
        if n > MAX_STRING {
            return Err(GgufError::TooLarge("chaîne"));
        }
        Ok(n)
    }
    fn string(&mut self) -> Result<String, GgufError> {
        let n = self.string_len()?;
        let mut buf = Vec::with_capacity(n.min(1 << 16) as usize);
        let got = (&mut self.r).take(n).read_to_end(&mut buf)?;
        if (got as u64) < n {
            return Err(GgufError::Truncated);
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    /// Taille fixe d'un type scalaire, `None` pour chaîne et tableau.
    fn fixed_size(t: u32) -> Result<Option<u64>, GgufError> {
        Ok(match t {
            0 | 1 | 7 => Some(1),
            2 | 3 => Some(2),
            4..=6 => Some(4),
            10..=12 => Some(8),
            8 | 9 => None,
            _ => return Err(GgufError::Io(format!("type de valeur inconnu {t}"))),
        })
    }

    fn scalar(&mut self, t: u32) -> Result<Value, GgufError> {
        Ok(match t {
            0 => Value::Int(self.bytes::<1>()?[0] as i128),
            1 => Value::Int(self.bytes::<1>()?[0] as i8 as i128),
            2 => Value::Int(u16::from_le_bytes(self.bytes()?) as i128),
            3 => Value::Int(i16::from_le_bytes(self.bytes()?) as i128),
            4 => Value::Int(self.u32()? as i128),
            5 => Value::Int(i32::from_le_bytes(self.bytes()?) as i128),
            6 => Value::Float(f32::from_le_bytes(self.bytes()?) as f64),
            7 => Value::Bool(self.bytes::<1>()?[0] != 0),
            10 => Value::Int(self.u64()? as i128),
            11 => Value::Int(i64::from_le_bytes(self.bytes()?) as i128),
            12 => Value::Float(f64::from_le_bytes(self.bytes()?)),
            8 => Value::Str(self.string()?),
            _ => return Err(GgufError::Io(format!("type de valeur inconnu {t}"))),
        })
    }

    fn value(&mut self, t: u32) -> Result<Value, GgufError> {
        if t != 9 {
            return self.scalar(t);
        }
        let et = self.u32()?;
        let len = self.u64()?;
        if len > MAX_ARRAY {
            return Err(GgufError::TooLarge("tableau"));
        }
        let is_int = matches!(et, 0..=5 | 10 | 11);
        if is_int && len <= KEEP_ARRAY {
            let mut ints = Vec::with_capacity(len as usize);
            for _ in 0..len {
                if let Value::Int(i) = self.scalar(et)? {
                    ints.push(i);
                }
            }
            return Ok(Value::Array { len, ints: Some(ints) });
        }
        match Self::fixed_size(et)? {
            Some(sz) => self.skip(sz * len)?,
            None if et == 8 => {
                for _ in 0..len {
                    let n = self.string_len()?;
                    self.skip(n)?;
                }
            }
            None => return Err(GgufError::Io("tableau de tableaux".into())),
        }
        Ok(Value::Array { len, ints: None })
    }
}

/// Lit l'en-tête et les métadonnées d'un flux GGUF.
pub fn parse(r: impl Read) -> Result<GgufMeta, GgufError> {
    let mut rd = Reader { r };
    if &rd.bytes::<4>()? != MAGIC {
        return Err(GgufError::NotGguf);
    }
    let version = rd.u32()?;
    if !(2..=3).contains(&version) {
        return Err(GgufError::Io(format!("version GGUF {version} non prise en charge")));
    }
    let _tensors = rd.u64()?;
    let kv_count = rd.u64()?;
    if kv_count > MAX_KV {
        return Err(GgufError::TooLarge("nombre de clés"));
    }
    let mut kv: HashMap<String, Value> = HashMap::new();
    for _ in 0..kv_count {
        let key = rd.string()?;
        let t = rd.u32()?;
        let v = rd.value(t)?;
        // Le gabarit de chat peut peser des dizaines de Ko : seule sa présence compte.
        let v = match (key.as_str(), v) {
            ("tokenizer.chat_template", Value::Str(_)) => Value::Bool(true),
            (_, v) => v,
        };
        kv.insert(key, v);
    }
    Ok(meta_from(&kv))
}

/// Lit les métadonnées d'un fichier local.
pub fn read_file(path: &Path) -> Result<GgufMeta, GgufError> {
    let f = std::fs::File::open(path).map_err(|e| GgufError::Io(e.to_string()))?;
    parse(io::BufReader::with_capacity(1 << 20, f))
}

fn meta_from(kv: &HashMap<String, Value>) -> GgufMeta {
    let s = |k: &str| match kv.get(k) {
        Some(Value::Str(v)) => Some(v.clone()),
        _ => None,
    };
    let arch = s("general.architecture").unwrap_or_default();
    let a = |suffix: &str| kv.get(&format!("{arch}.{suffix}")).and_then(Value::as_u64);
    let sampling = kv
        .iter()
        .filter_map(|(k, v)| Some((k.strip_prefix("general.sampling.")?.to_string(), v.as_f64()?)))
        .collect();
    GgufMeta {
        name: s("general.name"),
        size_label: s("general.size_label"),
        file_type: kv.get("general.file_type").and_then(Value::as_u64).map(|v| v as u32),
        context_length: a("context_length"),
        block_count: a("block_count"),
        embedding_length: a("embedding_length"),
        head_count: a("attention.head_count"),
        head_count_kv: a("attention.head_count_kv"),
        key_length: a("attention.key_length"),
        value_length: a("attention.value_length"),
        sliding_window: a("attention.sliding_window"),
        vocab_size: match kv.get("tokenizer.ggml.tokens") {
            Some(Value::Array { len, .. }) => Some(*len),
            _ => None,
        },
        has_chat_template: matches!(kv.get("tokenizer.chat_template"), Some(Value::Bool(true))),
        split_count: kv.get("split.count").and_then(Value::as_u64),
        sampling,
        arch,
    }
}

/// Projecteur de vision (mmproj) : architecture `clip`, ou « mmproj » dans le nom.
pub fn is_mmproj(meta: &GgufMeta, file_name: &str) -> bool {
    meta.arch == "clip"
        || file_name
            .rsplit('/')
            .next()
            .is_some_and(|n| n.to_ascii_lowercase().contains("mmproj"))
}
