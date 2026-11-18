use std::collections::HashMap;
use toml::Value;

type BindingResult = Result<HashMap<String, Value>, String>;

pub fn process_template<'a>(
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
