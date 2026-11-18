use std::collections::HashMap;
use toml::Value;

type BindingResult = Result<HashMap<String, Value>, String>;

pub fn process<'a>(
    vars: &'a Value,
    expand_template: &'a str,
) -> Result<impl Iterator<Item = Result<toml::Table, String>> + 'a, String> {
    Ok(evaluate_expr(vars)?.map(move |bindings_res| {
        expand_string(&expand_template, &bindings_res?)
            .parse::<toml::Table>()
            .map_err(|e| format!("Failed to parse expanded TOML: {}", e))
    }))
}

pub enum VarIter<'a> {
    Zip(ZipIter<'a>),
    Cross(CrossIter<'a>),
}

impl<'a> Iterator for VarIter<'a> {
    type Item = BindingResult;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            VarIter::Zip(iter) => iter.next(),
            VarIter::Cross(iter) => iter.next(),
        }
    }
}

#[derive(Clone)]
pub struct ZipIter<'a> {
    entries: Vec<(String, &'a Vec<Value>)>,
    index: usize,
    len: usize,
}

impl<'a> Iterator for ZipIter<'a> {
    type Item = BindingResult;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.len {
            let i = self.index;

            self.index += 1;

            Some(Ok(self.entries.iter().fold(
                HashMap::with_capacity(self.entries.len()),
                |mut acc, (formatted_key, arr)| {
                    acc.insert(formatted_key.clone(), arr[i].clone());
                    acc
                },
            )))
        } else {
            None
        }
    }
}

pub struct CrossIter<'a> {
    original_iters: Vec<ZipIter<'a>>,
    active_iters: Vec<ZipIter<'a>>,
    current_values: Vec<HashMap<String, Value>>,
    initialized: bool,
}

impl<'a> CrossIter<'a> {
    fn new(iters: Vec<ZipIter<'a>>) -> Self {
        let active_iters = iters.clone();

        Self {
            original_iters: iters,
            active_iters,
            current_values: Vec::new(),
            initialized: false,
        }
    }
}

impl<'a> Iterator for CrossIter<'a> {
    type Item = BindingResult;

    fn next(&mut self) -> Option<Self::Item> {
        if self.active_iters.is_empty() {
            return None;
        }

        if !self.initialized {
            self.initialized = true;
            let initial: Result<Vec<_>, String> = self
                .active_iters
                .iter_mut()
                .map(|iter| {
                    iter.next().unwrap_or_else(|| {
                        Err("Empty zip invalidates cross product".into())
                    })
                })
                .collect();

            match initial {
                Ok(vals) => self.current_values = vals,
                Err(e) => return Some(Err(e)),
            }
        } else {
            let mut advanced = false;

            for i in (0..self.active_iters.len()).rev() {
                match self.active_iters[i].next() {
                    Some(Ok(val)) => {
                        self.current_values[i] = val;
                        advanced = true;
                        break;
                    }
                    Some(Err(e)) => return Some(Err(e)),
                    None => {
                        self.active_iters[i] = self.original_iters[i].clone();
                        match self.active_iters[i].next() {
                            Some(Ok(val)) => {
                                self.current_values[i] = val;
                            }
                            Some(Err(e)) => return Some(Err(e)),
                            None => return None,
                        }
                    }
                }
            }

            if !advanced {
                return None;
            }
        }

        let total_len = self.current_values.iter().map(|m| m.len()).sum();

        Some(self.current_values
            .iter()
            .flat_map(|map| map.iter())
            .try_fold(
                HashMap::with_capacity(total_len),
                |mut acc, (k, v)| {
                    if acc.contains_key(k) {
                        Err(format!(
                            "Duplicate variable substitution name '{}' detected in cross product",
                            k
                        ))
                    } else {
                        acc.insert(k.clone(), v.clone());
                        Ok(acc)
                    }
                },
            ))
    }
}

pub fn evaluate_expr<'a>(val: &'a Value) -> Result<VarIter<'a>, String> {
    match val {
        Value::Table(table) => Ok(VarIter::Zip(zip_iter_from_table(table)?)),
        Value::Array(arr) => {
            let zip_iters = arr
                .iter()
                .map(|item| match item {
                    Value::Table(table) => zip_iter_from_table(table),
                    _ => Err("Each element in a top-level array must be a table".into()),
                })
                .collect::<Result<Vec<_>, String>>()?;

            Ok(VarIter::Cross(CrossIter::new(zip_iters)))
        }
        _ => Err("Variables definition must be a table (zip) or an array of tables (cross)".into()),
    }
}

fn zip_iter_from_table<'a>(
    table: &'a toml::value::Table,
) -> Result<ZipIter<'a>, String> {
    let entries = parse_zip_table(table)?;
    let len = validate_zip_lengths(&entries)?;

    Ok(ZipIter {
        entries,
        index: 0,
        len,
    })
}

fn parse_zip_table<'a>(
    table: &'a toml::value::Table,
) -> Result<Vec<(String, &'a Vec<Value>)>, String> {
    table
        .iter()
        .map(|(k, v)| {
            v.as_array()
                .map(|arr| (format!("${{{}}}", k), arr))
                .ok_or_else(|| {
                    format!("Value for variable '{}' must be an array", k)
                })
        })
        .collect()
}

fn validate_zip_lengths(
    entries: &[(String, &Vec<Value>)],
) -> Result<usize, String> {
    if entries.is_empty() {
        return Ok(0);
    }

    let len = entries[0].1.len();

    if let Some((k, arr)) = entries.iter().find(|(_, arr)| arr.len() != len) {
        Err(format!(
            "Mismatched array lengths in parallel group: '{}' has length {}, expected {}",
            k.trim_matches(|c| c == '$' || c == '{' || c == '}'), arr.len(), len
        ))
    } else {
        Ok(len)
    }
}

fn expand_string(template: &str, bindings: &HashMap<String, Value>) -> String {
    let mut result = String::with_capacity(template.len());
    let mut last_idx = 0;

    while let Some(start) = template[last_idx..].find("${") {
        let absolute_start = last_idx + start;
        result.push_str(&template[last_idx..absolute_start]);

        if let Some(end) = template[absolute_start..].find('}') {
            let absolute_end = absolute_start + end;
            let placeholder = &template[absolute_start..=absolute_end];

            if let Some(val) = bindings.get(placeholder) {
                result.push_str(&val.to_string());
            } else {
                result.push_str(placeholder);
            }
            last_idx = absolute_end + 1;
        } else {
            result.push_str(&template[absolute_start..]);
            last_idx = template.len();
            break;
        }
    }

    result.push_str(&template[last_idx..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(s: &str) -> Value {
        Value::Table(s.parse::<toml::Table>().unwrap())
    }

    fn vars_arr(s: &str) -> Value {
        // TOML documents must be tables, so the array is stored under "v".
        let mut t = s.parse::<toml::Table>().unwrap();

        t.remove("v").unwrap()
    }

    fn expand(v: &Value, tmpl: &str) -> Vec<Result<toml::Table, String>> {
        process(v, tmpl).unwrap().collect()
    }

    fn bindings(v: &Value) -> Vec<BindingResult> {
        evaluate_expr(v).unwrap().collect()
    }

    #[test]
    fn test_expand_string() {
        let b: HashMap<String, Value> = [
            ("${a}".to_string(), Value::Integer(5)),
            ("${s}".to_string(), Value::String("hi".into())),
        ]
        .into();

        assert_eq!(expand_string("", &b), "");
        assert_eq!(expand_string("no vars", &b), "no vars");
        assert_eq!(expand_string("x = ${a}", &b), "x = 5");
        assert_eq!(expand_string("${a}${a}", &b), "55");
        assert_eq!(expand_string("n = ${s}", &b), "n = \"hi\"");
        // Unknown placeholders are left untouched.
        assert_eq!(expand_string("${zz} ${a}", &b), "${zz} 5");
        // Unterminated placeholders are copied verbatim.
        assert_eq!(expand_string("a ${a", &b), "a ${a");
        assert_eq!(expand_string("${a} ${", &b), "5 ${");
        // A lone '$' or '{' isn't a placeholder.
        assert_eq!(expand_string("$ { a }", &b), "$ { a }");
    }

    #[test]
    fn test_expand_string_value_types() {
        let b: HashMap<String, Value> = [
            ("${f}".to_string(), Value::Float(1.5)),
            ("${b}".to_string(), Value::Boolean(true)),
            (
                "${l}".to_string(),
                Value::Array(vec![Value::Integer(1), Value::Integer(2)]),
            ),
        ]
        .into();

        assert_eq!(expand_string("${f} ${b} ${l}", &b), "1.5 true [1, 2]");
    }

    #[test]
    fn test_zip_basic() {
        let v = vars(
            r#"
            name = ["a", "b", "c"]
            id = [1, 2, 3]
            "#,
        );
        let res = bindings(&v)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(res.len(), 3);

        for (i, (n, id)) in [("a", 1), ("b", 2), ("c", 3)].iter().enumerate() {
            assert_eq!(res[i].len(), 2);
            assert_eq!(res[i]["${name}"], Value::String((*n).into()));
            assert_eq!(res[i]["${id}"], Value::Integer(*id));
        }
    }

    #[test]
    fn test_zip_empty_table() {
        assert!(bindings(&vars("")).is_empty());
    }

    #[test]
    fn test_zip_empty_arrays() {
        assert!(bindings(&vars("a = []\nb = []")).is_empty());
    }

    #[test]
    fn test_zip_mismatched_lengths() {
        let err = evaluate_expr(&vars("a = [1, 2]\nb = [1]")).err().unwrap();

        assert!(err.contains("Mismatched array lengths"), "{}", err);
        assert!(
            err.contains("length 1, expected 2")
                || err.contains("length 2, expected 1"),
            "{}",
            err
        );
        // The reported name is stripped of the "${}" decoration.
        assert!(!err.contains("${"), "{}", err);
    }

    #[test]
    fn test_zip_non_array_value() {
        let err = evaluate_expr(&vars("a = 5")).err().unwrap();

        assert_eq!(err, "Value for variable 'a' must be an array");
    }

    #[test]
    fn test_evaluate_expr_bad_top_level() {
        for v in [
            Value::Integer(1),
            Value::String("x".into()),
            Value::Boolean(false),
        ] {
            assert!(evaluate_expr(&v)
                .err()
                .unwrap()
                .contains("must be a table (zip) or an array of tables"));
        }

        let v = vars_arr("v = [1, 2]");

        assert_eq!(
            evaluate_expr(&v).err().unwrap(),
            "Each element in a top-level array must be a table"
        );
    }

    #[test]
    fn test_cross_basic_order() {
        // The last table varies fastest.
        let v = vars_arr(
            r#"
            v = [{ x = [1, 2] }, { y = ["a", "b", "c"] }]
            "#,
        );
        let res = bindings(&v)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let pairs = res
            .iter()
            .map(|m| {
                (m["${x}"].as_integer().unwrap(), m["${y}"].as_str().unwrap())
            })
            .collect::<Vec<_>>();

        assert_eq!(
            pairs,
            vec![(1, "a"), (1, "b"), (1, "c"), (2, "a"), (2, "b"), (2, "c")]
        );
    }

    #[test]
    fn test_cross_of_zips() {
        let v = vars_arr(
            r#"
            v = [
                { a = [1, 2], b = [10, 20] },
                { c = ["x", "y"] },
            ]
            "#,
        );
        let res = bindings(&v)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(res.len(), 4);
        assert!(res.iter().all(|m| m.len() == 3));

        let rows = res
            .iter()
            .map(|m| {
                (
                    m["${a}"].as_integer().unwrap(),
                    m["${b}"].as_integer().unwrap(),
                    m["${c}"].as_str().unwrap().to_string(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            rows,
            vec![
                (1, 10, "x".to_string()),
                (1, 10, "y".to_string()),
                (2, 20, "x".to_string()),
                (2, 20, "y".to_string()),
            ]
        );
    }

    #[test]
    fn test_cross_single_table() {
        let v = vars_arr("v = [{ x = [1, 2, 3] }]");
        let res = bindings(&v)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(
            res.iter()
                .map(|m| m["${x}"].as_integer().unwrap())
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn test_cross_three_way_count() {
        let v = vars_arr(
            "v = [{ a = [1, 2] }, { b = [1, 2, 3] }, { c = [1, 2, 3, 4] }]",
        );
        let res = bindings(&v)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut keys = res
            .iter()
            .map(|m| {
                (
                    m["${a}"].as_integer().unwrap(),
                    m["${b}"].as_integer().unwrap(),
                    m["${c}"].as_integer().unwrap(),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(keys.len(), 24);
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), 24);
    }

    #[test]
    fn test_cross_empty_array() {
        assert!(bindings(&Value::Array(vec![])).is_empty());
    }

    #[test]
    fn test_cross_empty_zip_is_error() {
        let v = vars_arr("v = [{ x = [1, 2] }, { y = [] }]");
        let res = bindings(&v);

        assert_eq!(
            res[0].as_ref().unwrap_err(),
            "Empty zip invalidates cross product"
        );
        assert_eq!(res.len(), 1);
    }

    #[test]
    fn test_cross_duplicate_names() {
        let v = vars_arr("v = [{ x = [1] }, { x = [2] }]");
        let res = bindings(&v);

        assert_eq!(res.len(), 1);
        assert!(res[0]
            .as_ref()
            .unwrap_err()
            .contains("Duplicate variable substitution name '${x}'"));
    }

    #[test]
    fn test_cross_mismatched_lengths_in_group() {
        let v = vars_arr("v = [{ x = [1] }, { a = [1, 2], b = [1] }]");

        assert!(evaluate_expr(&v)
            .err()
            .unwrap()
            .contains("Mismatched array lengths"));
    }

    #[test]
    fn test_process_template_zip() {
        let v = vars(
            r#"
            name = ["a", "b"]
            n = [1, 2]
            "#,
        );
        let res = expand(
            &v,
            r#"
            [device.${name}]
            index = ${n}
            label = ${name}
            "#,
        )
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
        let expected = ["a", "b"]
            .iter()
            .enumerate()
            .map(|(i, n)| {
                format!("[device.\"{n}\"]\nindex = {}\nlabel = \"{n}\"", i + 1)
                    .parse::<toml::Table>()
                    .unwrap()
            })
            .collect::<Vec<_>>();

        // Bare `${name}` expands to a quoted string, which is valid as a
        // quoted key.
        assert_eq!(res, expected);
    }

    #[test]
    fn test_process_template_cross() {
        let v = vars_arr(
            r#"v = [{ room = ["kitchen", "den"] }, { kind = ["temp", "hum"] }]"#,
        );
        let res = expand(&v, "room = ${room}\nkind = ${kind}\n")
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let got = res
            .iter()
            .map(|t| (t["room"].as_str().unwrap(), t["kind"].as_str().unwrap()))
            .collect::<Vec<_>>();

        assert_eq!(
            got,
            vec![
                ("kitchen", "temp"),
                ("kitchen", "hum"),
                ("den", "temp"),
                ("den", "hum")
            ]
        );
    }

    #[test]
    fn test_process_template_unquoted_substitution() {
        let v = vars("n = [1, 2, 3]");
        let res = expand(&v, "value = ${n}\nother = \"fixed\"")
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(res.len(), 3);

        for (i, t) in res.iter().enumerate() {
            assert_eq!(t["value"].as_integer(), Some(i as i64 + 1));
            assert_eq!(t["other"].as_str(), Some("fixed"));
        }
    }

    #[test]
    fn test_process_template_cross_values() {
        let v = vars_arr("v = [{ a = [1, 2] }, { b = [10, 20] }]");
        let res = expand(&v, "sum = [${a}, ${b}]")
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let got = res
            .iter()
            .map(|t| {
                t["sum"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_integer().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            got,
            vec![vec![1, 10], vec![1, 20], vec![2, 10], vec![2, 20]]
        );
    }

    #[test]
    fn test_process_template_invalid_toml() {
        let v = vars("n = [1]");
        let res = expand(&v, "value = = ${n}");

        assert_eq!(res.len(), 1);
        assert!(res[0]
            .as_ref()
            .unwrap_err()
            .starts_with("Failed to parse expanded TOML"));
    }

    #[test]
    fn test_process_template_unknown_placeholder_is_invalid_toml() {
        let v = vars("n = [1]");
        let res = expand(&v, "value = ${missing}");

        assert!(res[0]
            .as_ref()
            .unwrap_err()
            .starts_with("Failed to parse expanded TOML"));
    }

    #[test]
    fn test_process_template_propagates_binding_errors() {
        let v = vars_arr("v = [{ x = [1] }, { x = [2] }]");
        let res = expand(&v, "a = ${x}");

        assert_eq!(res.len(), 1);
        assert!(res[0].as_ref().unwrap_err().contains("Duplicate variable"));
    }

    #[test]
    fn test_process_template_bad_vars() {
        assert!(process(&Value::Integer(1), "a = 1").is_err());
        assert!(process(&vars("a = 1"), "a = 1").is_err());
    }

    #[test]
    fn test_process_template_no_vars_yields_nothing() {
        assert!(expand(&vars(""), "a = 1").is_empty());
    }
}
