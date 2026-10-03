//! 宽松标量解码 —— 让 Rust 的读取语义对齐基准 `load_config`。
//!
//! 基准是 Python，字段类型靠 `raw.get(k, default)` + `bool()` / `int()` /
//! `str()` 现场转换，所以它对"类型写歪了"的容忍度比 serde 高：
//!
//! | 写法 | 基准（Python） | 裸 serde |
//! |---|---|---|
//! | `auto_remote_name: "no"` | `bool("no")` → `true` | 报错 |
//! | `backup_keep: "7"` | `int("7")` → `7` | 报错 |
//! | `maven:`（null） | `_expand(None)` → `None` | 报错 |
//! | `restart_cmd: 123` | `str(123)` → `"123"` | 报错 |
//!
//! 这里提供 [`de_string`] / [`de_bool`] / [`de_i64`] / [`de_opt_string`]
//! 四个 `deserialize_with` 适配器，把上表逐条对齐。
//!
//! # 一处有意不照抄
//!
//! 基准对 null 字符串字段得到 `None`，写出 `key: null`；这里一律得到 `""`，
//! 写出 `key: ''`。两者在**所有** `if x:` / `if not x:` 判空处结果一致
//! （详见 `docs/MIGRATION.md`）。

use serde::de::{self, Deserialize, Deserializer, Visitor};

/// 任意 YAML 标量的类型擦除表示。
///
/// 之所以不直接用 `String`：`restart_cmd` 这类字段基准允许写成非字符串标量
/// （`str(value)`），而 `auto_remote_name` 基准用 `bool()` 接受任何标量。
/// 用一个 enum 把"原始类型"留到转换时再决定。
#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Str(String),
}

impl Scalar {
    /// 对应 Python `str(value)`。
    ///
    /// `str(True)` 是 `"True"` 不是 `"true"` —— 这条差异只有在用户把
    /// `restart_cmd: true` 写进配置时才会显形，但既然要做就做对。
    pub fn to_str_like(&self) -> String {
        match self {
            Scalar::Str(s) => s.clone(),
            Scalar::Bool(b) => (if *b { "True" } else { "False" }).to_owned(),
            Scalar::Int(i) => i.to_string(),
            Scalar::UInt(u) => u.to_string(),
            Scalar::Float(f) => fmt_float(*f),
        }
    }

    /// 对应 Python `bool(value)`（只覆盖标量）。
    pub fn to_bool(&self) -> bool {
        match self {
            Scalar::Bool(b) => *b,
            Scalar::Int(i) => *i != 0,
            Scalar::UInt(u) => *u != 0,
            Scalar::Float(f) => *f != 0.0,
            Scalar::Str(s) => !s.is_empty(),
        }
    }

    /// 对应 Python `int(value)`；解析不了返回 `None`（Python 这里会抛
    /// `ValueError`，我们让它变成反序列化错误，同样是"读不进来"）。
    pub fn to_i64(&self) -> Option<i64> {
        match self {
            Scalar::Int(i) => Some(*i),
            Scalar::UInt(u) => i64::try_from(*u).ok(),
            Scalar::Bool(b) => Some(i64::from(*b)),
            // Python int("7") = 7；int("7.0") 会抛 ValueError —— 对齐：不接受
            Scalar::Str(s) => s.trim().parse::<i64>().ok(),
            Scalar::Float(_) => None,
        }
    }
}

fn fmt_float(f: f64) -> String {
    if f == f.trunc() && f.is_finite() {
        format!("{f:.1}")
    } else {
        f.to_string()
    }
}

impl<'de> Deserialize<'de> for Scalar {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ScalarVisitor;

        impl<'de> Visitor<'de> for ScalarVisitor {
            type Value = Scalar;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("一个 YAML 标量")
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Scalar, E> {
                Ok(Scalar::Bool(v))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Scalar, E> {
                Ok(Scalar::Int(v))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Scalar, E> {
                Ok(Scalar::UInt(v))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Scalar, E> {
                Ok(Scalar::Float(v))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Scalar, E> {
                Ok(Scalar::Str(v.to_owned()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Scalar, E> {
                Ok(Scalar::Str(v))
            }
            fn visit_none<E: de::Error>(self) -> Result<Scalar, E> {
                Ok(Scalar::Str(String::new()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Scalar, E> {
                Ok(Scalar::Str(String::new()))
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Scalar, D::Error> {
                d.deserialize_any(ScalarVisitor)
            }
        }

        deserializer.deserialize_any(ScalarVisitor)
    }
}

/// `String` 字段：缺省由容器 `#[serde(default)]` 提供，`null` → `""`。
pub fn de_string<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<Scalar>::deserialize(d)?
        .map(|s| s.to_str_like())
        .unwrap_or_default())
}

/// `Option<String>` 字段：`null` → `None`，标量 → `Some(str(value))`。
///
/// 对应基准里三种不同写法的统一结果：
/// - `restart_cmd` 缺失 / `null` → `None`
/// - `git_branch` / `server` / `log_path` 的 `x or None`（空串 → `None`，
///   由调用方再 `.filter(|s| !s.is_empty())`）
/// - `password` / `key` 的 `raw.get(k)`（显式 `""` → `Some("")`）
pub fn de_opt_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<Scalar>::deserialize(d)?.map(|s| s.to_str_like()))
}

/// `bool` 字段：`null` → `false`，其余标量走 Python `bool()`。
pub fn de_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(Option::<Scalar>::deserialize(d)?
        .map(|s| s.to_bool())
        .unwrap_or(false))
}

/// `i64` 字段：接受整数与十进制数字串（Python `int("7")`）。
///
/// `null` → `0` 而不是报错：基准在这里是 `int(raw.get(k, 5))`，显式写
/// `backup_keep:` 会 `TypeError` 整份配置读不进来；我们选择宽松读入
/// （`docs/MIGRATION.md` 记录的有意宽容化之一）。真正非法的值仍然报错，
/// 和 Python `int("abc")` 一样读不进来。
pub fn de_i64<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    match Option::<Scalar>::deserialize(d)? {
        None => Ok(0),
        Some(s) => s.to_i64().ok_or_else(|| de::Error::custom("期望一个整数")),
    }
}

/// `Option<i64>` 字段：`null` → `None`，其余同 [`de_i64`]。
pub fn de_opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    Ok(Option::<Scalar>::deserialize(d)?.and_then(|s| s.to_i64()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(default)]
    struct Sample {
        #[serde(deserialize_with = "de_string")]
        s: String,
        #[serde(deserialize_with = "de_bool")]
        b: bool,
        #[serde(deserialize_with = "de_i64")]
        n: i64,
        #[serde(deserialize_with = "de_opt_string")]
        o: Option<String>,
    }

    impl Default for Sample {
        fn default() -> Self {
            Sample {
                s: "默认".into(),
                b: true,
                n: 7,
                o: None,
            }
        }
    }

    fn parse(text: &str) -> Result<Sample, serde_saphyr::Error> {
        serde_saphyr::from_str(text)
    }

    #[test]
    fn missing_fields_fall_back_to_container_default() {
        let s = parse("{}").unwrap();
        assert_eq!(s.s, "默认");
        assert!(s.b);
        assert_eq!(s.n, 7);
        assert_eq!(s.o, None);
    }

    #[test]
    fn null_string_becomes_empty_not_an_error() {
        // 基准写 `maven:` 也能读进来，Rust 这边不能因为一个 null 整份配置报废
        let s = parse("s:\nb:\nn:\no:").unwrap();
        assert_eq!(s.s, "");
        assert!(!s.b);
        assert_eq!(s.n, 0);
        assert_eq!(s.o, None);
    }

    #[test]
    fn python_truthiness_applies_to_bools() {
        assert!(parse("b: yes").unwrap().b);
        assert!(!parse("b: no").unwrap().b);
        // Python: bool("no") == True —— 基准就是这个语义
        assert!(parse("b: \"no\"").unwrap().b);
        assert!(parse("b: 1").unwrap().b);
        assert!(!parse("b: 0").unwrap().b);
    }

    #[test]
    fn int_accepts_digit_strings_like_python_int() {
        assert_eq!(parse("n: \"42\"").unwrap().n, 42);
        assert!(parse("n: abc").is_err());
        assert!(parse("n: 7.5").is_err());
    }

    #[test]
    fn non_string_scalars_coerce_to_string() {
        // 基准 _parse_restart_cmd 用 str(value)
        assert_eq!(parse("o: 123").unwrap().o.as_deref(), Some("123"));
        assert_eq!(parse("o: true").unwrap().o.as_deref(), Some("True"));
        assert_eq!(parse("o: \"\"").unwrap().o.as_deref(), Some(""));
    }
}
