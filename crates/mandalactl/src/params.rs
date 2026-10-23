// SPDX-License-Identifier: MPL-2.0

//! Arguments to parameters: how the words after a command name become
//! the flat JSON parameters of its request.
//!
//! **Three spellings, one meaning.** A bare word fills the verb's next
//! positional parameter (`act.action ZoomIn`); `name=value` names a
//! parameter and has its text coerced to the parameter's declared type
//! (`x=412` is the number 412, `text=412` the string); `name:=json`
//! names one and sends the JSON as written, for anything the first two
//! cannot say. Everything after a bare `--` is positional, which is
//! how a value that looks like `name=value` is sent as itself.
//!
//! **Types come from the server's vocabulary, from one of two
//! places.** A verb in `verbs::VERBS` carries its parameters' types; a
//! command the table has never heard of is typed from the connected
//! server's own `describe` ([`Signature::from_describe`]), which is
//! what lets a command that landed after this build be called with
//! proper numbers and booleans and no release of this crate.
//!
//! **Validation stops at the shape.** This module checks that text can
//! be the type it is declared as and that a name is a parameter the
//! command has. Whether a parameter is required, in range, or
//! combinable with another is the server's to say, in its own words
//! and with `data.param` — a second copy of those rules here could
//! only drift.

use serde_json::{Map, Number, Value};

use crate::verbs::{Shape, Verb};

/// One word of a command's arguments, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// A bare word: the next positional parameter's value.
    Positional(String),
    /// `name=value`: `value` is coerced to the parameter's type.
    Named { name: String, value: String },
    /// `name:=json`: `json` is sent as the JSON it parses as.
    Json { name: String, json: String },
}

/// Classify the words after a command name. A word is `name=value` or
/// `name:=json` only when the part before the sign is a plain
/// identifier (`[A-Za-z_][A-Za-z0-9_]*`), so a console line like
/// `section show` or `open a=b c` stays one positional word; after a
/// `--` nothing is classified at all.
pub fn tokenize(words: &[String]) -> Vec<Token> {
    let mut tokens = Vec::with_capacity(words.len());
    let mut literal = false;
    for word in words {
        if literal {
            tokens.push(Token::Positional(word.clone()));
        } else if word == "--" {
            literal = true;
        } else if let Some((name, json)) = word.split_once(":=").filter(|(name, _)| is_identifier(name)) {
            tokens.push(Token::Json {
                name: name.to_string(),
                json: json.to_string(),
            });
        } else if let Some((name, value)) = word.split_once('=').filter(|(name, _)| is_identifier(name)) {
            tokens.push(Token::Named {
                name: name.to_string(),
                value: value.to_string(),
            });
        } else {
            tokens.push(Token::Positional(word.clone()));
        }
    }
    tokens
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One parameter a command takes, owned so a signature can come from
/// either the static table or a `describe` reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamDef {
    pub name: String,
    pub shapes: Vec<Shape>,
    pub required: bool,
}

/// What argument-building needs to know about one command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    /// The command's name, for error messages.
    pub command: String,
    pub params: Vec<ParamDef>,
    /// Parameter names that may be given bare, in argv order.
    pub positional: Vec<String>,
    /// Whether `params` is the command's whole truth. A command the
    /// server has not described is not strict: a name is passed
    /// through as a string, for the server to judge.
    pub strict: bool,
}

impl Signature {
    /// The signature of a known verb.
    pub fn of(verb: &Verb) -> Signature {
        Signature {
            command: verb.name.to_string(),
            params: verb
                .params
                .iter()
                .map(|p| ParamDef {
                    name: p.name.to_string(),
                    shapes: p.ty.to_vec(),
                    required: p.required,
                })
                .collect(),
            positional: verb.positional.iter().map(|name| name.to_string()).collect(),
            strict: true,
        }
    }

    /// The signature `describe` reported for `entry` (one element of
    /// its `commands` array). No parameter is positional: which words
    /// read naturally bare is this crate's knowledge, and the server
    /// has none to give.
    pub fn from_describe(entry: &Value) -> Result<Signature, String> {
        let command = entry
            .get("name")
            .and_then(Value::as_str)
            .ok_or("describe sent a command with no name")?
            .to_string();
        let params = entry
            .get("params")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("describe sent `{command}` with no parameter list"))?
            .iter()
            .map(|p| {
                let name = p
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("describe sent a parameter of `{command}` with no name"))?;
                let shapes = p
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .split('|')
                    .map(Shape::from_word)
                    .collect();
                let required = p.get("required").and_then(Value::as_bool).unwrap_or(false);
                Ok(ParamDef {
                    name: name.to_string(),
                    shapes,
                    required,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Signature {
            command,
            params,
            positional: Vec::new(),
            strict: true,
        })
    }

    /// The signature of a command nobody has described: every name is
    /// passed through, and nothing is positional.
    pub fn unknown(command: &str) -> Signature {
        Signature {
            command: command.to_string(),
            params: Vec::new(),
            positional: Vec::new(),
            strict: false,
        }
    }

    fn param(&self, name: &str) -> Option<&ParamDef> {
        self.params.iter().find(|p| p.name == name)
    }

    fn valid_names(&self) -> String {
        if self.params.is_empty() {
            return "it takes no parameters".to_string();
        }
        let names: Vec<&str> = self.params.iter().map(|p| p.name.as_str()).collect();
        format!("its parameters are {}", names.join(", "))
    }
}

/// Build the parameter object for a request from classified words.
/// `Err` is a usage error: it names the word at fault and what would
/// have been accepted.
pub fn build_params(signature: &Signature, tokens: &[Token]) -> Result<Map<String, Value>, String> {
    let mut params = Map::new();
    let command = &signature.command;
    // A parameter given by name is not also a bare word's to fill, so
    // `scene.hit_test x=5 412` reads 412 as `y`.
    let named: Vec<&str> = tokens
        .iter()
        .filter_map(|token| match token {
            Token::Named { name, .. } | Token::Json { name, .. } => Some(name.as_str()),
            Token::Positional(_) => None,
        })
        .collect();
    let mut bare = signature
        .positional
        .iter()
        .filter(|name| !named.contains(&name.as_str()));
    for token in tokens {
        let (name, value) = match token {
            Token::Positional(text) => {
                let Some(name) = bare.next() else {
                    return Err(positional_overflow(signature, text));
                };
                let def = signature.param(name);
                (name.as_str(), coerce_param(name, def, text)?)
            }
            Token::Named { name, value } => {
                let def = known_param(signature, name)?;
                (name.as_str(), coerce_param(name, def, value)?)
            }
            Token::Json { name, json } => {
                known_param(signature, name)?;
                let parsed = serde_json::from_str(json)
                    .map_err(|e| format!("`{name}:=` is not valid JSON ({e}): {json}"))?;
                (name.as_str(), parsed)
            }
        };
        if name == "id" || name == "cmd" {
            return Err(format!(
                "`{name}` is an envelope key, not a parameter of `{command}` (format/ipc.md, the reserved-word rule)"
            ));
        }
        if params.insert(name.to_string(), value).is_some() {
            return Err(format!("`{name}` is given twice for `{command}`"));
        }
    }
    Ok(params)
}

fn known_param<'a>(signature: &'a Signature, name: &str) -> Result<Option<&'a ParamDef>, String> {
    let def = signature.param(name);
    if def.is_none() && signature.strict {
        return Err(format!(
            "`{}` has no parameter `{name}`; {}",
            signature.command,
            signature.valid_names()
        ));
    }
    Ok(def)
}

fn positional_overflow(signature: &Signature, word: &str) -> String {
    let command = &signature.command;
    let takes = if signature.positional.is_empty() {
        format!("`{command}` takes no bare arguments; pass parameters as name=value")
    } else {
        format!(
            "`{command}` takes {} bare argument(s) ({}); pass the rest as name=value",
            signature.positional.len(),
            signature.positional.join(", ")
        )
    };
    format!("unexpected argument `{word}`: {takes}. To send a value that looks like name=value, put it after `--`")
}

fn coerce_param(name: &str, def: Option<&ParamDef>, text: &str) -> Result<Value, String> {
    let shapes = def.map_or(&[][..], |d| d.shapes.as_slice());
    coerce(text, shapes).map_err(|why| format!("`{name}`: {why}"))
}

/// Turn argument text into a JSON value of one of `shapes`.
///
/// The structured shapes win over `string` when the text is
/// unambiguously one — it parses as a number, is `true` / `false`,
/// opens with `{` or `[` — and `string` takes whatever is left. So
/// `act.action`'s `string|object` makes `ZoomIn` a string and
/// `{"OpenDocument":"m.json"}` an object. No shapes (an undescribed
/// parameter) is the same rule with every shape allowed but `string`
/// as the fallback. An array also reads a bare comma list as strings
/// (`mods=ctrl,shift`), and the empty text as the empty array, which
/// for `mods` is "hold none", not "leave as is".
pub fn coerce(text: &str, shapes: &[Shape]) -> Result<Value, String> {
    // A shape this build cannot name decides nothing when `string` is
    // also admitted: `text=412` stays the string it was typed as.
    if shapes.is_empty() || (shapes.contains(&Shape::Other) && !shapes.contains(&Shape::String)) {
        return Ok(serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())));
    }
    let admits = |shape: Shape| shapes.contains(&shape);
    let trimmed = text.trim();
    if admits(Shape::Boolean) {
        match trimmed {
            "true" => return Ok(Value::Bool(true)),
            "false" => return Ok(Value::Bool(false)),
            _ => {}
        }
    }
    if admits(Shape::Integer) {
        if let Ok(n) = trimmed.parse::<i64>() {
            return Ok(Value::Number(n.into()));
        }
        if let Ok(n) = trimmed.parse::<u64>() {
            return Ok(Value::Number(n.into()));
        }
    }
    if admits(Shape::Number) {
        if let Ok(n) = trimmed.parse::<i64>() {
            return Ok(Value::Number(n.into()));
        }
        if let Some(n) = trimmed.parse::<f64>().ok().and_then(Number::from_f64) {
            return Ok(Value::Number(n));
        }
    }
    if admits(Shape::Object) && trimmed.starts_with('{') {
        return match serde_json::from_str::<Value>(trimmed) {
            Ok(object @ Value::Object(_)) => Ok(object),
            Ok(_) => Err(format!("expected a JSON object, got {text}")),
            Err(e) => Err(format!("not valid JSON ({e}): {text}")),
        };
    }
    if admits(Shape::Array) {
        if trimmed.starts_with('[') {
            return match serde_json::from_str::<Value>(trimmed) {
                Ok(array @ Value::Array(_)) => Ok(array),
                Ok(_) => Err(format!("expected a JSON array, got {text}")),
                Err(e) => Err(format!("not valid JSON ({e}): {text}")),
            };
        }
        if trimmed.starts_with('{') && !admits(Shape::Object) && !admits(Shape::String) {
            return Err(format!(
                "expected a JSON array or a comma-separated list, got an object: {text}"
            ));
        }
        if !admits(Shape::String) {
            return Ok(Value::Array(
                trimmed
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(|item| Value::String(item.to_string()))
                    .collect(),
            ));
        }
    }
    if admits(Shape::String) {
        return Ok(Value::String(text.to_string()));
    }
    let wanted: Vec<&str> = shapes.iter().map(|s| s.word()).collect();
    Err(format!("expected {}, got `{text}`", wanted.join(" or ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::verbs::{lookup, VERBS};
    use serde_json::json;

    fn words(line: &[&str]) -> Vec<String> {
        line.iter().map(|w| w.to_string()).collect()
    }

    fn built(verb: &str, line: &[&str]) -> Map<String, Value> {
        let signature = Signature::of(lookup(verb).expect("a known verb"));
        build_params(&signature, &tokenize(&words(line))).expect("builds")
    }

    fn refused(verb: &str, line: &[&str]) -> String {
        let signature = Signature::of(lookup(verb).expect("a known verb"));
        build_params(&signature, &tokenize(&words(line))).expect_err("a usage error")
    }

    #[test]
    fn test_tokenize_classifies_the_three_spellings() {
        assert_eq!(
            tokenize(&words(&["ZoomIn", "x=412", "region:={\"x\":1}"])),
            vec![
                Token::Positional("ZoomIn".into()),
                Token::Named {
                    name: "x".into(),
                    value: "412".into()
                },
                Token::Json {
                    name: "region".into(),
                    json: "{\"x\":1}".into()
                },
            ]
        );
    }

    #[test]
    fn test_tokenize_keeps_prose_with_an_equals_sign_as_one_positional() {
        for prose in ["set a=b", "fps = on", "=x", "9lives=1", "a-b=1", "a b:=1"] {
            assert_eq!(
                tokenize(&words(&[prose])),
                vec![Token::Positional(prose.into())],
                "{prose:?} must not read as a named parameter"
            );
        }
    }

    #[test]
    fn test_tokenize_treats_everything_after_double_dash_as_positional() {
        assert_eq!(
            tokenize(&words(&["--", "fps=on", "x:=1", "--"])),
            vec![
                Token::Positional("fps=on".into()),
                Token::Positional("x:=1".into()),
                Token::Positional("--".into()),
            ]
        );
    }

    #[test]
    fn test_bare_words_fill_positionals_in_order_and_coerce_to_the_declared_type() {
        let params = built("scene.hit_test", &["412", "96.5", "space=surface"]);
        assert_eq!(params["x"], json!(412));
        assert_eq!(params["y"], json!(96.5));
        assert_eq!(params["space"], json!("surface"));
    }

    #[test]
    fn test_a_string_parameter_stays_a_string_whatever_it_looks_like() {
        let params = built("scene.find_text", &["412", "exact=true"]);
        assert_eq!(params["text"], json!("412"));
        assert_eq!(params["exact"], json!(true));
        let params = built("input.key", &["true"]);
        assert_eq!(params["key"], json!("true"));
    }

    #[test]
    fn test_string_or_object_reads_a_variant_name_or_a_json_object() {
        assert_eq!(built("act.action", &["Undo"])["action"], json!("Undo"));
        assert_eq!(
            built("act.action", &["{\"OpenDocument\":\"m.mindmap.json\"}"])["action"],
            json!({"OpenDocument": "m.mindmap.json"})
        );
    }

    #[test]
    fn test_a_malformed_json_object_is_not_demoted_to_a_string() {
        let err = refused("act.action", &["{\"OpenDocument\":"]);
        assert!(
            err.contains("`action`") && err.contains("not valid JSON"),
            "{err}"
        );
    }

    #[test]
    fn test_arrays_read_json_or_a_comma_list_and_empty_means_empty() {
        assert_eq!(
            built("input.key", &["z", "mods=ctrl,shift"])["mods"],
            json!(["ctrl", "shift"])
        );
        assert_eq!(
            built("input.key", &["z", "mods=ctrl, shift"])["mods"],
            json!(["ctrl", "shift"])
        );
        assert_eq!(
            built("input.key", &["z", "mods=[\"alt\"]"])["mods"],
            json!(["alt"])
        );
        assert_eq!(
            built("input.key", &["z", "mods="])["mods"],
            json!([]),
            "[] holds no modifier"
        );
        assert_eq!(
            built(
                "act.macro",
                &["steps=[{\"kind\":\"ConsoleLine\",\"line\":\"fps on\"}]"]
            )["steps"],
            json!([{"kind": "ConsoleLine", "line": "fps on"}])
        );
    }

    #[test]
    fn test_integers_and_numbers_are_told_apart() {
        assert_eq!(built("query.undo", &["5"])["limit"], json!(5));
        assert_eq!(built("input.wheel", &["-3"])["dy"], json!(-3));
        assert_eq!(built("input.wheel", &["0.5"])["dy"], json!(0.5));
        let err = refused("query.undo", &["5.5"]);
        assert!(err.contains("`limit`") && err.contains("integer"), "{err}");
        let err = refused("scene.hit_test", &["abc", "1"]);
        assert!(err.contains("`x`") && err.contains("number"), "{err}");
    }

    #[test]
    fn test_booleans_accept_only_true_and_false() {
        assert_eq!(built("capture.screenshot", &["fit=false"])["fit"], json!(false));
        let err = refused("capture.screenshot", &["fit=yes"]);
        assert!(err.contains("`fit`") && err.contains("boolean"), "{err}");
    }

    #[test]
    fn test_json_spelling_sends_the_json_as_written() {
        let params = built(
            "capture.screenshot",
            &["region:={\"x\":0,\"y\":0,\"w\":10,\"h\":10}", "width:=640"],
        );
        assert_eq!(params["region"], json!({"x": 0, "y": 0, "w": 10, "h": 10}));
        assert_eq!(params["width"], json!(640));
        let err = refused("capture.screenshot", &["width:=6x"]);
        assert!(
            err.contains("`width:=`") && err.contains("not valid JSON"),
            "{err}"
        );
    }

    #[test]
    fn test_an_object_parameter_must_be_an_object() {
        let err = refused("capture.screenshot", &["region={}x"]);
        assert!(err.contains("`region`"), "{err}");
        let err = refused("capture.screenshot", &["region=oops"]);
        assert!(err.contains("`region`") && err.contains("object"), "{err}");
    }

    #[test]
    fn test_a_name_the_command_lacks_is_refused_with_the_names_it_has() {
        let err = refused("scene.hit_test", &["regoin=x"]);
        assert!(err.contains("regoin") && err.contains("x, y, space"), "{err}");
        let err = refused("ping", &["x=1"]);
        assert!(err.contains("takes no parameters"), "{err}");
    }

    #[test]
    fn test_too_many_bare_words_name_the_escape_hatch() {
        let err = refused("act.console", &["section", "show"]);
        assert!(
            err.contains("`show`") && err.contains("line") && err.contains("--"),
            "{err}"
        );
        let err = refused("ping", &["hello"]);
        assert!(err.contains("no bare arguments"), "{err}");
    }

    #[test]
    fn test_a_parameter_given_twice_is_refused() {
        let err = refused("query.undo", &["limit=5", "limit=6"]);
        assert!(err.contains("`limit`") && err.contains("twice"), "{err}");
        let err = refused("query.undo", &["limit=5", "limit:=6"]);
        assert!(err.contains("`limit`") && err.contains("twice"), "{err}");
    }

    #[test]
    fn test_bare_words_fill_only_the_positionals_not_given_by_name() {
        let params = built("scene.hit_test", &["x=5", "412"]);
        assert_eq!((params["x"].clone(), params["y"].clone()), (json!(5), json!(412)));
        let params = built("input.drag", &["from_x=1", "2", "3", "4"]);
        assert_eq!(
            [
                &params["from_x"],
                &params["from_y"],
                &params["to_x"],
                &params["to_y"]
            ],
            [&json!(1), &json!(2), &json!(3), &json!(4)]
        );
        let err = refused("query.undo", &["limit=6", "5"]);
        assert!(err.contains("unexpected argument `5`"), "{err}");
        let err = refused("scene.hit_test", &["1", "2", "x=3"]);
        assert!(err.contains("unexpected argument `2`"), "{err}");
    }

    #[test]
    fn test_a_type_word_from_the_future_does_not_override_string() {
        let shapes = [Shape::String, Shape::Other];
        assert_eq!(coerce("412", &shapes), Ok(json!("412")));
        assert_eq!(coerce("412", &[Shape::Other]), Ok(json!(412)));
        assert_eq!(coerce("412", &[]), Ok(json!(412)));
    }

    #[test]
    fn test_an_object_where_only_an_array_is_wanted_is_refused_not_split() {
        let err = refused("act.macro", &["steps={\"kind\":\"Action\"}"]);
        assert!(err.contains("`steps`") && err.contains("JSON array"), "{err}");
        assert_eq!(
            built("scene.dump", &["kinds=nodes,edges"])["kinds"],
            json!(["nodes", "edges"])
        );
    }

    #[test]
    fn test_envelope_keys_are_never_parameters() {
        let lenient = Signature::unknown("future.thing");
        for word in ["id=1", "cmd=x", "id:=1"] {
            let err = build_params(&lenient, &tokenize(&words(&[word]))).expect_err("reserved");
            assert!(err.contains("envelope"), "{word}: {err}");
        }
    }

    #[test]
    fn test_console_line_after_double_dash_is_one_value() {
        let params = built("act.console", &["--", "fps=on"]);
        assert_eq!(params["line"], json!("fps=on"));
        let params = built("act.console", &["open maps/a=b.mindmap.json"]);
        assert_eq!(params["line"], json!("open maps/a=b.mindmap.json"));
    }

    #[test]
    fn test_a_described_command_is_typed_from_describe_and_has_nothing_positional() {
        let entry = json!({
            "name": "clock.step", "family": "clock", "summary": "s",
            "params": [
                {"name": "ms", "type": "integer", "required": false, "doc": "d"},
                {"name": "mode", "type": "string|object", "required": false, "doc": "d"},
                {"name": "wat", "type": "duration", "required": false, "doc": "d"}
            ],
            "result": []
        });
        let signature = Signature::from_describe(&entry).expect("a signature");
        let params = build_params(
            &signature,
            &tokenize(&words(&["ms=100", "mode=virtual", "wat=3"])),
        )
        .expect("builds");
        assert_eq!(params["ms"], json!(100));
        assert_eq!(params["mode"], json!("virtual"));
        assert_eq!(params["wat"], json!(3), "a type word from the future is inferred");
        let err = build_params(&signature, &tokenize(&words(&["100"]))).expect_err("nothing is positional");
        assert!(err.contains("name=value"), "{err}");
        let err = build_params(&signature, &tokenize(&words(&["frames=1"]))).expect_err("not a parameter");
        assert!(err.contains("ms, mode, wat"), "{err}");
    }

    #[test]
    fn test_describe_entries_without_the_expected_keys_are_refused() {
        assert!(Signature::from_describe(&json!({})).is_err());
        assert!(Signature::from_describe(&json!({"name": "x"})).is_err());
        assert!(Signature::from_describe(&json!({"name": "x", "params": [{}]})).is_err());
    }

    #[test]
    fn test_an_undescribed_command_passes_names_through_for_the_server_to_judge() {
        let signature = Signature::unknown("future.thing");
        let params = build_params(
            &signature,
            &tokenize(&words(&["n=3", "flag=true", "s=hello", "o:={\"a\":1}"])),
        )
        .expect("builds");
        assert_eq!(params["n"], json!(3));
        assert_eq!(params["flag"], json!(true));
        assert_eq!(params["s"], json!("hello"));
        assert_eq!(params["o"], json!({"a": 1}));
        let err = build_params(&signature, &tokenize(&words(&["bare"]))).expect_err("nothing is positional");
        assert!(err.contains("no bare arguments"), "{err}");
    }

    #[test]
    fn test_every_verb_signature_is_strict_and_lists_its_params() {
        for verb in VERBS {
            let signature = Signature::of(verb);
            assert!(signature.strict);
            assert_eq!(signature.params.len(), verb.params.len());
        }
    }
}
