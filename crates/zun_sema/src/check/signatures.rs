use crate::check::*;

impl Checker<'_> {
    /// Whether a function declaration receives a hidden `this`: it opens its
    /// parameter list with `this`, or its body mentions `this` (the
    /// constructor-function and borrowed-method idiom, `function F() { this.x
    /// = 1 }`), so `new F()` and `F.call(obj)` have a receiver to bind.
    fn declaration_takes_this(&self, f: &ast::Function) -> bool {
        self.declaration_opens_with_this(f)
            || self
                .source
                .get(f.body.span.start as usize..f.body.span.end as usize)
                // The comment- and string-aware scan closures already use. A
                // bare word split counted `this` inside a string literal or a
                // comment, so an error message mentioning 'this' silently gave
                // the function a hidden receiver parameter -- and a call site
                // that did not know failed IL verification with StackUnderflow.
                .is_some_and(crate::check::expr::literal::mentions_this_word)
    }

    /// Whether a function declaration's parameter list opens with `this`.
    fn declaration_opens_with_this(&self, f: &ast::Function) -> bool {
        let Some(text) = self
            .source
            .get(f.name.span.end as usize..f.body.span.start as usize)
        else {
            return false;
        };
        let Some(open) = text.find('(') else {
            return false;
        };
        text[open + 1..]
            .trim_start()
            .strip_prefix("this")
            .is_some_and(|after| {
                !after.starts_with(|c: char| c.is_alphanumeric() || c == '_' || c == '$')
            })
    }

    pub(in crate::check) fn collect_signatures(&mut self, program: &ast::Program) {
        // Register every top-level name before resolving annotations. This
        // permits a function-typed parameter to refer to a function declared
        // later in the source file.
        for (index, f) in program.functions.iter().enumerate() {
            let name = self.text(&f.name).to_string();
            if self.by_name.insert(name, index as u32).is_some() {
                self.error(f.name.span, "duplicate function declaration");
            }
        }
        for f in &program.functions {
            let name = self.text(&f.name).to_string();
            if matches!(f.body.statements.first(), Some(ast::Stmt::Let { name, .. }) if self.text(name) == "arguments")
            {
                self.arguments_functions
                    .insert(self.signatures.len() as u32);
            }
            // A leading `this: T` parameter is the explicit receiver. The
            // parser drops it from `params`, so the signature carries it as a
            // hidden first parameter (as a closure does) and the emitted name
            // gets the `@this` marker the runtime dispatches on.
            let takes_this = self.declaration_takes_this(f);
            let mut params: Vec<Ty> = Vec::new();
            if takes_this {
                params.push(Ty::Any);
                self.receiver_funcs.insert(self.signatures.len() as u32);
            }
            params.extend(f.params.iter().map(|p| match &p.ty {
                // An unannotated parameter is untyped (`any`), as in JS/TS
                // without `--noImplicitAny`; types are erased anyway.
                Some(t) => self.resolve_annotation(t),
                None => Ty::Any,
            }));
            let return_ty = match &f.return_type {
                Some(t) => self.resolve_annotation(t),
                // An unannotated return type is `any` when the body returns a
                // value, else `void` — so a value-returning JS function is not
                // forced to `void` (which would reject every `return x`).
                None => {
                    if f.is_async {
                        Ty::Promise
                    } else if block_returns_value(&f.body) {
                        Ty::Any
                    } else {
                        Ty::Void
                    }
                }
            };
            let optional_count = {
                let mut count = 0;
                for (index, p) in f.params.iter().enumerate().rev() {
                    if p.is_rest {
                        continue;
                    }
                    if p.default_value.is_some()
                        || params.get(index + usize::from(takes_this)) == Some(&Ty::Any)
                    {
                        count += 1;
                    } else {
                        break;
                    }
                }
                count
            };
            self.signatures.push(Signature {
                // The `@async` marker, like `@this`, rides on the declared
                // name: an async function is **not** a constructor and owns no
                // `prototype`, and nothing else at run time can tell one apart
                // -- zun compiles it as a generator (`is_generator: is_async ||
                // is_generator`), so the body scan `method_yields` uses answers
                // the same for both, and an empty body defeats that scan
                // entirely. Markers are stripped as *suffixes*, never at the
                // first `@`: a closure name legitimately contains one
                // (`<closure@123>`).
                name: {
                    let name = if takes_this {
                        format!("{name}@this")
                    } else {
                        name
                    };
                    let name = if f.is_async {
                        format!("{name}@async")
                    } else {
                        name
                    };
                    // A generator needs its own marker for the same reason: a
                    // body with no `yield` is indistinguishable from a plain
                    // function by the scan, and `new` on one must throw.
                    if f.is_generator {
                        format!("{name}@generator")
                    } else {
                        name
                    }
                },
                params,
                return_ty,
                // Trailing defaulted parameters, and trailing dynamic (`any`) ones —
                // JS lets any argument be omitted, which reads as `undefined`.
                optional_count,
                default_count: f
                    .params
                    .iter()
                    .rev()
                    .filter(|p| !p.is_rest)
                    .take_while(|p| p.default_value.is_some())
                    .count(),
                rest_param: f
                    .params
                    .iter()
                    .position(|parameter| parameter.is_rest)
                    .map(|index| index + usize::from(takes_this)),
                is_generator: f.is_generator,
                is_static: false,
            });
        }

        // Method signatures follow the top-level functions, in (class, method)
        // order. Instance methods have an implicit leading `this`; static
        // methods intentionally do not.
        self.methods_by_class = vec![HashMap::new(); program.classes.len()];
        self.static_twins = vec![HashMap::new(); program.classes.len()];
        // Owned names avoid borrowing `self` across the loop's `&mut self`
        // calls (the same set was previously rescanned per method).
        let classes_used_as_base: HashSet<String> = program
            .classes
            .iter()
            .filter_map(|class| class.base.as_ref().map(|base| self.text(base).to_owned()))
            .collect();
        for (class_idx, class) in program.classes.iter().enumerate() {
            for m in &class.methods {
                let base_name = self.text(&m.name).to_string();
                let name = match m.accessor {
                    Some(ast::AccessorKind::Get) => format!("get_{base_name}"),
                    Some(ast::AccessorKind::Set) => format!("set_{base_name}"),
                    None => base_name,
                };
                let receiver_ty = match self.text(&class.name) {
                    "String" => Ty::String,
                    "Number" => Ty::F64,
                    "Array" => Ty::Array,
                    "Map" => Ty::Map,
                    "Set" => Ty::Set,
                    "WeakMap" => Ty::WeakMap,
                    "WeakSet" => Ty::WeakSet,
                    "WeakRef" => Ty::WeakRef,
                    "RegExp" => Ty::RegExp,
                    "BigInt" => Ty::BigInt,
                    "FinalizationRegistry" => Ty::FinalizationRegistry,
                    // A native-backed class: the receiver is the native value
                    // (an array, a buffer), never a fixed-layout instance. Its
                    // constructor still runs on the typed construction shell.
                    _ if self.native_base_of(class_idx as u32).is_some()
                        && self.text(&m.name) != "constructor" =>
                    {
                        Ty::Any
                    }
                    _ => Ty::Instance(class_idx as u32),
                };
                // A static method that mentions `this`/`super` receives the
                // receiving class value as a hidden first parameter.
                let hidden_class_param = m.is_static
                    && !m.is_extern
                    && self
                        .source
                        .get(m.body.span.start as usize..m.body.span.end as usize)
                        .is_some_and(|text| {
                            text.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                                .any(|word| word == "this" || word == "super")
                        });
                let mut params = if m.is_static {
                    if hidden_class_param {
                        vec![Ty::Any]
                    } else {
                        Vec::new()
                    }
                } else {
                    vec![receiver_ty]
                };
                for p in &m.params {
                    params.push(match &p.ty {
                        Some(t) => self.resolve_annotation(t),
                        // An untyped parameter is dynamic (`any`), matching JS.
                        None => Ty::Any,
                    });
                }
                let fluent = m.return_type.is_none()
                    && m.accessor.is_none()
                    && !m.is_static
                    && !m.is_async
                    && !m.is_generator
                    && !classes_used_as_base.contains(self.text(&class.name))
                    && self.block_returns_only_this(&m.body);
                let return_ty = match &m.return_type {
                    Some(t) => self.resolve_annotation(t),
                    None => {
                        if fluent && self.native_base_of(class_idx as u32).is_none() {
                            Ty::Instance(class_idx as u32)
                        } else if fluent || block_returns_value(&m.body) {
                            Ty::Any
                        } else {
                            Ty::Void
                        }
                    }
                };
                let func_index = self.signatures.len() as u32;
                let mut twin_static = false;
                match self.methods_by_class[class_idx].get(&name).copied() {
                    None => {
                        self.methods_by_class[class_idx].insert(name.clone(), func_index);
                    }
                    // A static and an instance method of one name live on
                    // different objects and do not collide. The instance one
                    // keeps the main slot; the static one is its twin.
                    Some(existing)
                        if self.signatures[existing as usize].is_static != m.is_static
                            && !self.static_twins[class_idx].contains_key(&name) =>
                    {
                        let (instance, statik) = if m.is_static {
                            (existing, func_index)
                        } else {
                            (func_index, existing)
                        };
                        self.methods_by_class[class_idx].insert(name.clone(), instance);
                        self.static_twins[class_idx].insert(name.clone(), statik);
                        twin_static = m.is_static;
                        // The vtable is slotted by emitted name, so the two
                        // must not share one: the static twin is emitted as
                        // `name@static`. When the static came first it was
                        // already registered under the plain name.
                        if !m.is_static {
                            let earlier = &mut self.signatures[statik as usize].name;
                            *earlier = match earlier.strip_suffix("@this") {
                                Some(plain) => format!("{plain}@static@this"),
                                None => format!("{earlier}@static"),
                            };
                        }
                    }
                    Some(_) => {
                        self.error(m.name.span, format!("duplicate method `{name}`"));
                    }
                }
                let name = if twin_static { format!("{name}@static") } else { name };
                let name = if hidden_class_param {
                    self.static_this.insert(func_index, class_idx as u32);
                    format!("{name}@this")
                } else {
                    name
                };
                // Computed before `params` is moved into the signature.
                let optional_count = {
                    let hidden = params.len().saturating_sub(m.params.len());
                    let mut count = 0;
                    for (index, p) in m.params.iter().enumerate().rev() {
                        if p.is_rest {
                            continue;
                        }
                        if p.default_value.is_some() || params.get(index + hidden) == Some(&Ty::Any)
                        {
                            count += 1;
                        } else {
                            break;
                        }
                    }
                    count
                };
                self.signatures.push(Signature {
                    name,
                    params,
                    return_ty,
                    // Trailing defaulted parameters, and trailing dynamic
                    // (`any`) ones -- the same permissiveness the free-function
                    // path has, because JS requires no argument at any arity.
                    // Without the `any` clause an inherited
                    // `constructor(first: any, second: any, ...rest: any[])`
                    // reported `expects at least 2 fixed argument(s)` for
                    // `new Level4("only")`, which node accepts. `params` carries
                    // the hidden receiver first, so the type lookup is offset.
                    optional_count,
                    default_count: m
                        .params
                        .iter()
                        .rev()
                        .filter(|p| !p.is_rest)
                        .take_while(|p| p.default_value.is_some())
                        .count(),
                    rest_param: m
                        .params
                        .iter()
                        .position(|parameter| parameter.is_rest)
                        .map(|index| index + 1),
                    is_generator: m.is_generator,
                    is_static: m.is_static,
                });
            }
        }

        // Computed-symbol-key methods follow all vtable methods, in (class,
        // method) order, so every existing func index is unchanged. Each is
        // modeled as a static function (no implicit `this`) and recorded for
        // the module-initializer side-table registration.
        for (class_idx, class) in program.classes.iter().enumerate() {
            for sm in &class.symbol_methods {
                // A non-static symbol method receives the instance as an implicit
                // leading `this` (flagged by the `@this` name suffix, which the
                // interpreter's `with_this_arg` keys on); a static one has none.
                let takes_this = !sm.function.is_static;
                let mut params = if takes_this {
                    if self.native_base_of(class_idx as u32).is_some() {
                        vec![Ty::Any]
                    } else {
                        vec![Ty::Instance(class_idx as u32)]
                    }
                } else {
                    Vec::new()
                };
                for p in &sm.function.params {
                    params.push(match &p.ty {
                        Some(t) => self.resolve_annotation(t),
                        // An untyped parameter is dynamic (`any`), matching JS.
                        None => Ty::Any,
                    });
                }
                let return_ty = match &sm.function.return_type {
                    Some(t) => self.resolve_annotation(t),
                    None => {
                        if block_returns_value(&sm.function.body) {
                            Ty::Any
                        } else {
                            Ty::Void
                        }
                    }
                };
                let func_index = self.signatures.len() as u32;
                self.signatures.push(Signature {
                    name: if takes_this {
                        format!("[symbol]@{class_idx}@this")
                    } else {
                        format!("[symbol]@{class_idx}")
                    },
                    params,
                    return_ty,
                    optional_count: sm
                        .function
                        .params
                        .iter()
                        .rev()
                        .take_while(|p| p.default_value.is_some())
                        .count(),
                    default_count: sm
                        .function
                        .params
                        .iter()
                        .rev()
                        .filter(|p| !p.is_rest)
                        .take_while(|p| p.default_value.is_some())
                        .count(),
                    rest_param: sm
                        .function
                        .params
                        .iter()
                        .position(|parameter| parameter.is_rest)
                        .map(|index| index + usize::from(takes_this)),
                    is_generator: false,
                    is_static: !takes_this,
                });
                match self.well_known_symbol_of_key(&sm.key) {
                    // An instance `get [Symbol.x]()` is an accessor property of the
                    // class prototype, not a method: the initializer installs it as
                    // such. (A static getter, e.g. `static get [Symbol.species]()`,
                    // keeps the function-valued registration the runtime reads.)
                    Some(symbol)
                        if sm.function.accessor == Some(ast::AccessorKind::Get)
                            && !sm.function.is_static =>
                    {
                        self.class_symbol_getters.push((
                            class_idx as u32,
                            symbol.id().get(),
                            func_index,
                        ));
                    }
                    Some(symbol) => {
                        self.class_symbol_methods.push((
                            class_idx as u32,
                            symbol.id().get(),
                            func_index,
                        ));
                        if symbol == zun_core::WellKnownSymbol::HasInstance {
                            self.classes_with_has_instance.insert(class_idx as u32);
                        }
                        if symbol == zun_core::WellKnownSymbol::Iterator {
                            self.classes_with_iterator.insert(class_idx as u32);
                        }
                    }
                    None => self.error(
                        sm.key.span(),
                        "a class computed member key must be a well-known Symbol \
                         (e.g. `Symbol.hasInstance`, `Symbol.iterator`)",
                    ),
                }
            }
        }
    }

    /// Recognize a computed class-member key that is a well-known Symbol
    /// (`Symbol.hasInstance`, `Symbol.iterator`, …).
    /// `re[Symbol.replace|match|matchAll|split|search](str, ...)` on a RegExp
    /// value (a literal, or any expression statically typed `RegExp`).
    pub(in crate::check) fn is_regex_symbol_call(
        &mut self,
        cx: &mut FunctionCx,
        callee: &ast::Expr,
        args: &[ast::Expr],
    ) -> bool {
        let (ast::Expr::Index { object, index, .. }, Some(first)) = (callee, args.first()) else {
            return false;
        };
        if matches!(first, ast::Expr::Spread { .. }) {
            return false;
        }
        if !matches!(
            self.well_known_symbol_of_key(index),
            Some(zun_core::WellKnownSymbol::Split)
                | Some(zun_core::WellKnownSymbol::Replace)
                | Some(zun_core::WellKnownSymbol::Match)
                | Some(zun_core::WellKnownSymbol::Search)
                | Some(zun_core::WellKnownSymbol::MatchAll)
        ) {
            return false;
        }
        matches!(object.as_ref(), ast::Expr::RegexLiteral { .. })
            || self.check_expr(cx, object).ty == Ty::RegExp
    }

    pub(in crate::check) fn well_known_symbol_of_key(
        &self,
        key: &ast::Expr,
    ) -> Option<zun_core::WellKnownSymbol> {
        let (object, field) = match key {
            ast::Expr::Field { object, field, .. } => {
                let ast::Expr::Ident(name) = object.as_ref() else {
                    return None;
                };
                (self.text(name), self.text(field))
            }
            ast::Expr::StaticField { class, field, .. } => (self.text(class), self.text(field)),
            _ => return None,
        };
        if object != "Symbol" || self.by_name.contains_key("Symbol") {
            return None;
        }
        zun_core::WellKnownSymbol::from_member(field)
    }
}
