use super::*;
use scr_ast::ClassDeclaration;
use std::{
    cell::{Cell, OnceCell, RefCell},
    collections::HashMap,
};

// Runtime declarations share method bodies with escaped and direct invocations.
// Keep the parser's public AST independent of runtime ownership.
pub(crate) struct ClassDefinition {
    pub name: scr_ast::Binding,
    pub parent: Option<TypeAnnotation>,
    pub interfaces: Vec<TypeAnnotation>,
    pub fields: Vec<scr_ast::FieldDeclaration>,
    pub methods: Vec<Rc<value::UserDeclaration>>,
}
impl From<&ClassDeclaration> for ClassDefinition {
    fn from(declaration: &ClassDeclaration) -> Self {
        Self {
            name: declaration.name.clone(),
            parent: declaration.parent.clone(),
            interfaces: declaration.interfaces.clone(),
            fields: declaration.fields.clone(),
            methods: declaration
                .methods
                .iter()
                .cloned()
                .map(value::UserDeclaration::new)
                .map(Rc::new)
                .collect(),
        }
    }
}

pub(crate) struct ScriptClass {
    pub declaration: ClassDefinition,
    pub is_interface: bool,
    pub builtin: bool,
    pub parent: OnceCell<Option<Rc<ScriptClass>>>,
    pub resolving: Cell<bool>,
    pub interfaces: OnceCell<Vec<Rc<ScriptClass>>>,
    pub method_cache: crate::method_cache::MethodCache,
    pub environment: usize,
    pub owner: Rc<()>,
    pub source: Option<Arc<SourceFile>>,
}
impl ScriptClass {
    fn cached_method(
        &self,
        name: &str,
        span: Span,
    ) -> Option<(&Self, &Rc<value::UserDeclaration>)> {
        let hint = self.method_cache.hint(span)?;
        let mut owner = self;
        for _ in 0..hint.depth {
            owner = owner.parent.get().and_then(Option::as_deref)?;
        }
        let declaration = owner.declaration.methods.get(hint.method)?;
        if declaration.name != name {
            return None;
        }
        self.method_cache.hit();
        Some((owner, declaration))
    }

    pub fn is_a(&self, class: &Rc<Self>) -> bool {
        std::ptr::eq(self, class.as_ref())
            || self
                .parent
                .get()
                .and_then(Option::as_ref)
                .is_some_and(|parent| parent.is_a(class))
    }
    pub fn is_exception(&self) -> bool {
        self.builtin
            || self
                .parent
                .get()
                .and_then(Option::as_ref)
                .is_some_and(|parent| parent.is_exception())
    }
    fn fields(&self) -> impl Iterator<Item = (&Self, &scr_ast::FieldDeclaration)> {
        // Resolved ordinary declarations already provide an immutable slot
        // layout. Inherited exception fields keep their linear ancestor walk.
        let direct = self.parent.get().is_some_and(Option::is_none);
        let mut ancestors = Vec::new();
        let mut class = (!direct).then_some(self);
        while let Some(base) = class {
            ancestors.push(base);
            class = base.parent.get().and_then(Option::as_deref);
        }
        direct
            .then_some(self)
            .into_iter()
            .chain(ancestors.into_iter().rev())
            .flat_map(|class| {
                class
                    .declaration
                    .fields
                    .iter()
                    .map(move |field| (class, field))
            })
    }
    fn method_owner(&self, name: &str) -> Option<&Self> {
        if self
            .declaration
            .methods
            .iter()
            .any(|method| method.name == name)
        {
            Some(self)
        } else {
            self.parent
                .get()
                .and_then(Option::as_ref)
                .and_then(|parent| parent.method_owner(name))
        }
    }
}
struct FieldSize {
    bytes: usize,
    has_handles: bool,
}
enum FieldValues<'a> {
    Pending(&'a HashMap<String, Value>),
    // Construction appends initialized fields in declaration order. Missing
    // typed fields have no value, so consume this stream only on a name match.
    PendingSlots(&'a [(String, Value)]),
    Live(usize),
}

enum ResolvedProperty {
    Method(Function),
    Value(Value),
}

pub(crate) struct ScriptObject {
    pub class: Rc<ScriptClass>,
    pub scope: usize,
    pub(super) payload: Cell<usize>,
    // Sticky: shared/native handles can expose counts during host measurement.
    // Exceptions also retain snapshots for hidden fields/diagnostic ownership.
    pub(super) requires_snapshot: Cell<bool>,
    pub exception: RefCell<Option<exceptions::ExceptionData>>,
}
impl std::fmt::Debug for ScriptObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(&self.class.declaration.name.name)
            .finish_non_exhaustive()
    }
}
impl NativeObject for ScriptObject {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn type_name(&self) -> &str {
        &self.class.declaration.name.name
    }
    fn payload_bytes(&self) -> usize {
        self.payload.get().saturating_add(
            self.exception
                .borrow()
                .as_ref()
                .map_or(0, |data| data.payload_bytes()),
        )
    }
    fn get_property(&self, _: &str) -> std::result::Result<Value, String> {
        Err("script objects require their owning runtime".into())
    }
}

impl Runtime {
    pub(super) fn declare_class(
        &mut self,
        declaration: &ClassDeclaration,
        span: Span,
    ) -> Result<()> {
        self.declare_type(declaration, false, span)
    }

    pub(super) fn declare_interface(
        &mut self,
        declaration: &scr_ast::InterfaceDeclaration,
        span: Span,
    ) -> Result<()> {
        self.declare_type(
            &ClassDeclaration {
                name: declaration.name.clone(),
                parent: None,
                interfaces: Vec::new(),
                fields: Vec::new(),
                methods: declaration.methods.clone(),
            },
            true,
            span,
        )
    }

    fn declare_type(
        &mut self,
        declaration: &ClassDeclaration,
        is_interface: bool,
        span: Span,
    ) -> Result<()> {
        self.checkpoint(span)?;
        if exceptions::is_builtin(&declaration.name.name)
            || [
                "int", "float", "string", "bytes", "bool", "list", "map", "callable", "void",
            ]
            .contains(&declaration.name.name.as_str())
        {
            return self.fail(
                "class name conflicts with a built-in type",
                declaration.name.span,
            );
        }
        if self
            .environments
            .scope(self.environments.current)
            .classes
            .contains_key(&declaration.name.name)
        {
            return self.fail(
                format!("class {} is already declared", declaration.name.name),
                span,
            );
        }
        let mut names = std::collections::HashSet::new();
        for name in declaration
            .fields
            .iter()
            .map(|f| &f.binding.name)
            .chain(declaration.methods.iter().map(|f| &f.name))
        {
            if name == "this" || !names.insert(name) {
                return self.fail("invalid or duplicate class member", span);
            }
        }
        for method in &declaration.methods {
            self.checkpoint(method.name_span)?;
            self.validate_function(method)?;
        }
        if is_interface
            && declaration
                .methods
                .iter()
                .any(|m| m.name == "__construct" || !m.body.statements.is_empty())
        {
            return self.fail(
                "interfaces contain method signatures, without constructors",
                span,
            );
        }
        let class = Rc::new(ScriptClass {
            declaration: declaration.into(),
            is_interface,
            builtin: false,
            parent: OnceCell::new(),
            resolving: Cell::new(false),
            interfaces: OnceCell::new(),
            method_cache: crate::method_cache::MethodCache::default(),
            environment: self.environments.current,
            owner: self.identity.clone(),
            source: self.source.clone(),
        });
        self.environments
            .scope_mut(self.environments.current)
            .classes
            .insert(declaration.name.name.clone(), class.clone());
        self.environments.capture();
        Ok(())
    }

    pub(super) fn check_class(&mut self, name: &str, span: Span) -> Result<()> {
        let class = self.class(None, name).expect("declared class");
        self.ensure_implementation(&class, span)
    }

    pub(super) fn ensure_implementation(&mut self, class: &ScriptClass, span: Span) -> Result<()> {
        if class.interfaces.get().is_some() {
            return Ok(());
        }
        if self.class_resolution_depth >= MAX_RECURSION_LIMIT {
            return self.fail("exception inheritance exceeds 64 classes", span);
        }
        if class.resolving.replace(true) {
            return self.fail("cyclic exception inheritance", span);
        }
        self.class_resolution_depth += 1;
        let previous = self.environments.current;
        let previous_source = self.source.clone();
        self.environments.current = class.environment;
        self.source = class.source.clone();
        let result = (|| {
            if class.parent.get().is_none() {
                let parent = if let Some(annotation) = &class.declaration.parent {
                    let (namespace, name) = annotation
                        .name
                        .split_once("::")
                        .map_or((None, annotation.name.as_str()), |(ns, name)| {
                            (Some(ns), name)
                        });
                    let parent = self.class(namespace, name).ok_or_else(|| {
                        self.native_error(
                            NativeError::new(
                                "ClassError",
                                format!("undefined exception base {}", annotation.name),
                            ),
                            annotation.span,
                        )
                    })?;
                    self.ensure_implementation(&parent, annotation.span)?;
                    if !parent.is_exception() {
                        return self.fail("extends requires an exception class", annotation.span);
                    }
                    let mut depth = 1;
                    let mut ancestor = Some(parent.as_ref());
                    while let Some(base) = ancestor {
                        self.checkpoint(span)?;
                        depth += 1;
                        if depth > MAX_RECURSION_LIMIT {
                            return self.fail("exception inheritance exceeds 64 classes", span);
                        }
                        ancestor = base.parent.get().and_then(Option::as_deref);
                    }
                    for field in &class.declaration.fields {
                        if exceptions::PROPERTIES.contains(&field.binding.name.as_str())
                            || parent
                                .fields()
                                .any(|(_, inherited)| inherited.binding.name == field.binding.name)
                            || parent.method_owner(&field.binding.name).is_some()
                        {
                            return self.fail(
                                "exception field conflicts with an inherited member",
                                field.binding.span,
                            );
                        }
                    }
                    for method in &class.declaration.methods {
                        if exceptions::PROPERTIES.contains(&method.name.as_str())
                            || parent
                                .fields()
                                .any(|(_, field)| field.binding.name == method.name)
                        {
                            return self.fail(
                                "exception method conflicts with an inherited property",
                                method.name_span,
                            );
                        }
                    }
                    Some(parent)
                } else {
                    None
                };
                let _ = class.parent.set(parent);
            }
            let mut interfaces = Vec::new();
            for annotation in &class.declaration.interfaces {
                self.checkpoint(annotation.span)?;
                let (namespace, name) = annotation
                    .name
                    .split_once("::")
                    .map_or((None, annotation.name.as_str()), |(ns, name)| {
                        (Some(ns), name)
                    });
                let interface = self
                    .class(namespace, name)
                    .filter(|class| class.is_interface)
                    .ok_or_else(|| {
                        self.native_error(
                            NativeError::new(
                                "ClassError",
                                format!("undefined interface {}", annotation.name),
                            ),
                            annotation.span,
                        )
                    })?;
                if interfaces.iter().any(|other| Rc::ptr_eq(other, &interface)) {
                    return self.fail("duplicate implemented interface", annotation.span);
                }
                interfaces.push(interface);
            }
            if let Some(parent) = class.parent.get().and_then(Option::as_ref) {
                for interface in parent.interfaces.get().into_iter().flatten() {
                    if !interfaces.iter().any(|other| Rc::ptr_eq(other, interface)) {
                        interfaces.push(interface.clone());
                    }
                }
            }
            self.validate_implementation(class, &interfaces, span)?;
            let _ = class.interfaces.set(interfaces);
            Ok(())
        })();
        self.environments.current = previous;
        self.source = previous_source;
        class.resolving.set(false);
        self.class_resolution_depth -= 1;
        result
    }

    fn same_annotation(
        &mut self,
        left: &Option<TypeAnnotation>,
        left_env: usize,
        right: &Option<TypeAnnotation>,
        right_env: usize,
    ) -> bool {
        let (Some(left), Some(right)) = (left, right) else {
            return left.is_none() && right.is_none();
        };
        let previous = self.environments.current;
        let mut resolve = |annotation: &TypeAnnotation, env| {
            self.environments.current = env;
            let (namespace, name) = annotation
                .name
                .split_once("::")
                .map_or((None, annotation.name.as_str()), |(ns, name)| {
                    (Some(ns), name)
                });
            self.class(namespace, name)
        };
        let left_type = resolve(left, left_env);
        let right_type = resolve(right, right_env);
        self.environments.current = previous;
        match (left_type, right_type) {
            (Some(left), Some(right)) => Rc::ptr_eq(&left, &right),
            (None, None) => left.name == right.name,
            _ => false,
        }
    }

    fn validate_implementation(
        &mut self,
        class: &ScriptClass,
        interfaces: &[Rc<ScriptClass>],
        span: Span,
    ) -> Result<()> {
        for interface in interfaces {
            for required in &interface.declaration.methods {
                self.checkpoint(span)?;
                let owner = class.method_owner(&required.name);
                let method = owner
                    .as_ref()
                    .and_then(|owner| {
                        owner
                            .declaration
                            .methods
                            .iter()
                            .find(|m| m.name == required.name)
                    })
                    .ok_or_else(|| {
                        self.native_error(
                            NativeError::new(
                                "ClassError",
                                format!(
                                    "class {} must implement {}->{}",
                                    class.declaration.name.name,
                                    interface.declaration.name.name,
                                    required.name
                                ),
                            ),
                            span,
                        )
                    })?;
                let mut matches = method.parameters.len() == required.parameters.len()
                    && self.same_annotation(
                        &method.return_type,
                        owner.expect("method owner").environment,
                        &required.return_type,
                        interface.environment,
                    );
                for (actual, expected) in method.parameters.iter().zip(&required.parameters) {
                    self.checkpoint(actual.span)?;
                    matches &= actual.variadic == expected.variadic
                        && actual.default.is_some() == expected.default.is_some()
                        && self.same_annotation(
                            &actual.annotation,
                            owner.expect("method owner").environment,
                            &expected.annotation,
                            interface.environment,
                        );
                }
                if !matches {
                    let previous_source = self.source.clone();
                    self.source = owner.expect("method owner").source.clone();
                    let error = self.native_error(
                        NativeError::new(
                            "ClassError",
                            format!(
                                "method {}->{} does not match interface {}",
                                class.declaration.name.name,
                                method.name,
                                interface.declaration.name.name
                            ),
                        ),
                        method.name_span,
                    );
                    self.source = previous_source;
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    pub(super) fn class(&self, namespace: Option<&str>, name: &str) -> Option<Rc<ScriptClass>> {
        if let Some(namespace) = namespace {
            return self
                .environments
                .find_alias(namespace)
                .and_then(|alias| self.environments.scope(alias.scope).classes.get(name))
                .cloned();
        }
        let mut id = Some(self.environments.current);
        while let Some(current) = id {
            let scope = self.environments.scope(current);
            if let Some(class) = scope.classes.get(name) {
                return Some(class.clone());
            }
            id = scope.parent;
        }
        if [
            "int", "float", "string", "bytes", "bool", "list", "map", "callable", "void", "null",
        ]
        .contains(&name)
        {
            return None;
        }
        if exceptions::is_builtin(name) {
            Some(self.builtin_exception_class(name))
        } else {
            self.exception_classes.borrow().get(name).cloned()
        }
    }

    fn checked_object<'a>(
        &self,
        object: &'a Rc<dyn NativeObject>,
        span: Span,
    ) -> Result<Option<&'a ScriptObject>> {
        let object = object
            .as_any()
            .and_then(|object| object.downcast_ref::<ScriptObject>());
        if object.is_some_and(|o| !Rc::ptr_eq(&o.class.owner, &self.identity)) {
            return self.fail("object belongs to a different runtime", span);
        }
        Ok(object)
    }

    fn bound_method(&mut self, function: Function) -> Value {
        let function = Rc::new(function);
        self.environments.track_function(&function);
        Value::Callable(Callable(function))
    }

    // Resolve before evaluating arguments. Fields (including callable fields)
    // and native properties retain their ordinary property-access semantics.
    fn resolve_property(
        &self,
        object: &Rc<dyn NativeObject>,
        name: &str,
        span: Span,
        cache: bool,
    ) -> Result<ResolvedProperty> {
        let Some(instance) = self.checked_object(object, span)? else {
            return self
                .property_result(object.get_property(name), span)
                .map(ResolvedProperty::Value);
        };
        let class = &instance.class;
        if instance.exception.borrow().is_some() && exceptions::PROPERTIES.contains(&name) {
            return self
                .object_field(object, name, span)
                .map(ResolvedProperty::Value);
        }
        let cache = cache && class.method_cache.enabled(span);
        #[cfg(test)]
        let cache = cache && !self.disable_method_cache;
        let cached = cache.then(|| class.cached_method(name, span)).flatten();
        let (owner, declaration) = if let Some(method) = cached {
            method
        } else {
            if class.fields().any(|(_, f)| f.binding.name == name) {
                return self
                    .object_field(object, name, span)
                    .map(ResolvedProperty::Value);
            }
            let owner = class.method_owner(name).ok_or_else(|| {
                self.native_error(
                    NativeError::new("PropertyError", format!("unknown method {name}")),
                    span,
                )
            })?;
            let (method, declaration) = owner
                .declaration
                .methods
                .iter()
                .enumerate()
                .find(|(_, m)| m.name == name)
                .expect("resolved method owner");
            if cache {
                let mut depth = 0;
                let mut current = class.as_ref();
                while !std::ptr::eq(current, owner) {
                    current = current
                        .parent
                        .get()
                        .and_then(Option::as_deref)
                        .expect("resolved method owner");
                    depth += 1;
                }
                class.method_cache.remember(crate::method_cache::Hint {
                    span,
                    depth,
                    method,
                });
            }
            (owner, declaration)
        };
        Ok(ResolvedProperty::Method(Function::User {
            declaration: Rc::clone(declaration),
            environment: owner.environment,
            owner: self.identity.clone(),
            source: owner.source.clone(),
            receiver: Some(Value::NativeObject(object.clone())),
        }))
    }

    pub(super) fn call_property(
        &mut self,
        base: &Expr,
        name: &str,
        arguments: &[Expr],
        property_span: Span,
        call_span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        let Value::NativeObject(object) = self.evaluate(base, output)? else {
            return self.fail("property access requires a native object", property_span);
        };
        let function = match self.resolve_property(&object, name, property_span, true)? {
            ResolvedProperty::Method(function) => function,
            ResolvedProperty::Value(callee) => {
                self.validate_value(&callee, 0, property_span)?;
                return self.call(callee, arguments, call_span, output);
            }
        };
        // Like a bound callable, a direct method accounts for one Value slot
        // before argument evaluation, without allocating a callable identity.
        if self
            .options
            .max_value_bytes
            .is_some_and(|limit| std::mem::size_of::<Value>() > limit)
        {
            return self.fail("execution value byte limit exceeded", property_span);
        }
        let Function::User { environment, .. } = &function else {
            unreachable!()
        };
        // The receiver remains an external object root via `function`. Its class
        // environment can differ from an inherited method's lexical environment,
        // so protect the latter explicitly across argument evaluation and entry.
        self.environments.temporary_roots.push(*environment);
        let result = (|| {
            let values = self.evaluate_arguments(arguments, call_span, output)?;
            self.call_function(&function, values, call_span, output)
        })();
        self.environments.temporary_roots.pop();
        result
    }

    pub(super) fn object_property(
        &mut self,
        object: &Rc<dyn NativeObject>,
        name: &str,
        span: Span,
    ) -> Result<Value> {
        match self.resolve_property(object, name, span, false)? {
            ResolvedProperty::Method(function) => Ok(self.bound_method(function)),
            ResolvedProperty::Value(value) => Ok(value),
        }
    }

    pub(super) fn object_field(
        &self,
        object: &Rc<dyn NativeObject>,
        name: &str,
        span: Span,
    ) -> Result<Value> {
        if let Some(instance) = self.checked_object(object, span)? {
            if name == "trace" {
                self.check_exception_trace(instance, span)?;
            }
            if let Some(value) = self.exception_property(instance, name) {
                return Ok(value);
            }
            let Some(slot) = instance
                .class
                .fields()
                .position(|(_, f)| f.binding.name == name)
            else {
                return Err(self.native_error(
                    NativeError::new("PropertyError", format!("unknown field {name}")),
                    span,
                ));
            };
            return self
                .environments
                .scope(instance.scope)
                .variables
                .get_slot(slot, name)
                .cloned()
                .ok_or_else(|| {
                    self.native_error(
                        NativeError::new("PropertyError", format!("field {name} is uninitialized")),
                        span,
                    )
                });
        }
        self.property_result(object.get_property(name), span)
    }

    // Object references occupy handles, rather than recursively expanding cycles.
    fn field_measure<const TRACK_HANDLES: bool>(
        &self,
        value: &Value,
        depth: usize,
        span: Span,
    ) -> Result<FieldSize> {
        if self
            .options
            .interrupt_flag
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            return Err(Signal::Interrupted(
                self.error("execution interrupted", span),
            ));
        }
        if depth > MAX_VALUE_DEPTH {
            return self.fail("value nesting exceeds 128 levels", span);
        }
        let mut bytes = std::mem::size_of::<Value>();
        let mut has_handles = false;
        let add = |left: usize, right: usize| {
            left.checked_add(right)
                .ok_or_else(|| Signal::Error(self.error("object size overflow", span)))
        };
        match value {
            Value::String(value) => bytes = add(bytes, value.len())?,
            Value::Bytes(value) => {
                if TRACK_HANDLES {
                    has_handles = true;
                }
                bytes = add(bytes, value.len())?;
            }
            Value::NativeObject(value) => {
                if TRACK_HANDLES {
                    has_handles = true;
                }
                if value.as_any().is_none_or(|o| !o.is::<ScriptObject>()) {
                    bytes = add(bytes, value.payload_bytes())?;
                }
            }
            Value::List(values) => {
                for value in values {
                    let size = self.field_measure::<TRACK_HANDLES>(value, depth + 1, span)?;
                    bytes = add(bytes, size.bytes)?;
                    if TRACK_HANDLES {
                        has_handles |= size.has_handles;
                    }
                }
            }
            Value::Map(values) => {
                for (key, value) in values {
                    bytes = add(bytes, std::mem::size_of::<MapKey>())?;
                    if let MapKey::String(key) = key {
                        bytes = add(bytes, key.len())?;
                    }
                    let size = self.field_measure::<TRACK_HANDLES>(value, depth + 1, span)?;
                    bytes = add(bytes, size.bytes)?;
                    if TRACK_HANDLES {
                        has_handles |= size.has_handles;
                    }
                }
            }
            _ => {}
        }
        if self
            .options
            .max_value_bytes
            .is_some_and(|limit| bytes > limit)
        {
            return self.fail("execution value byte limit exceeded", span);
        }
        Ok(FieldSize { bytes, has_handles })
    }

    fn object_size<const TRACK_HANDLES: bool>(
        &mut self,
        class: &ScriptClass,
        values: FieldValues<'_>,
        replacement: Option<(&str, &Value)>,
        span: Span,
    ) -> Result<FieldSize> {
        let mut bytes = std::mem::size_of::<Value>();
        let mut has_handles = false;
        let mut pending = match &values {
            FieldValues::PendingSlots(values) => Some(values.iter().peekable()),
            _ => None,
        };
        for (slot, (_, field)) in class.fields().enumerate() {
            self.checkpoint(span)?;
            let original = match &values {
                FieldValues::Pending(values) => values.get(&field.binding.name),
                FieldValues::PendingSlots(_) => pending.as_mut().and_then(|values| {
                    values
                        .next_if(|(name, _)| name == &field.binding.name)
                        .map(|(_, value)| value)
                }),
                FieldValues::Live(scope) => self
                    .environments
                    .scope(*scope)
                    .variables
                    .get_slot(slot, &field.binding.name),
            };
            let value = replacement
                .filter(|(name, _)| *name == field.binding.name)
                .map(|(_, v)| v)
                .or(original);
            let size = if let Some(value) = value {
                self.field_measure::<TRACK_HANDLES>(value, 1, span)?
            } else {
                FieldSize {
                    bytes: std::mem::size_of::<Value>(),
                    has_handles: false,
                }
            };
            if TRACK_HANDLES {
                has_handles |= size.has_handles;
            }
            bytes = bytes
                .checked_add(field.binding.name.len())
                .and_then(|n| n.checked_add(size.bytes))
                .ok_or_else(|| Signal::Error(self.error("object size overflow", span)))?;
            if self
                .options
                .max_value_bytes
                .is_some_and(|limit| bytes > limit)
            {
                return self.fail("execution value byte limit exceeded", span);
            }
        }
        Ok(FieldSize {
            bytes: bytes - std::mem::size_of::<Value>(),
            has_handles,
        })
    }

    pub(super) fn set_object_field(
        &mut self,
        object: &Rc<dyn NativeObject>,
        name: &str,
        value: Value,
        span: Span,
    ) -> Result<()> {
        if let Some(instance) = self.checked_object(object, span)? {
            let class = instance.class.clone();
            let scope = instance.scope;
            if instance.exception.borrow().is_some() && exceptions::PROPERTIES.contains(&name) {
                return Err(self.native_error(
                    NativeError::new("PropertyError", format!("property {name} is read-only")),
                    span,
                ));
            }
            let (slot, (owner, field)) = class
                .fields()
                .enumerate()
                .find(|(_, (_, f))| f.binding.name == name)
                .ok_or_else(|| {
                    self.native_error(
                        NativeError::new("PropertyError", format!("unknown field {name}")),
                        span,
                    )
                })?;
            let previous = self.environments.current;
            self.environments.current = owner.environment;
            let annotation = self.annotation(&field.annotation, &value, span, false);
            self.environments.current = previous;
            annotation?;
            let track_handles = !instance.requires_snapshot.get();
            let snapshot = !track_handles;
            #[cfg(test)]
            let snapshot = snapshot || self.force_field_snapshot;
            let values =
                snapshot.then(|| self.environments.scope(scope).variables.clone().into_map());
            let fields = match &values {
                Some(values) => FieldValues::Pending(values),
                None => FieldValues::Live(scope),
            };
            // No stored-value borrow crosses a checkpoint: object_size reborrows
            // after it, and the receiver remains a collector root throughout.
            let measured = if track_handles {
                self.object_size::<true>(&class, fields, Some((name, &value)), span)?
            } else {
                self.object_size::<false>(
                    &class,
                    FieldValues::Pending(values.as_ref().expect("shared handles retain snapshot")),
                    Some((name, &value)),
                    span,
                )?
            };
            let payload = measured.bytes;
            if self.options.max_value_bytes.is_some_and(|limit| {
                payload
                    .saturating_add(
                        instance
                            .exception
                            .borrow()
                            .as_ref()
                            .map_or(0, |data| data.payload_bytes()),
                    )
                    .saturating_add(std::mem::size_of::<Value>())
                    > limit
            }) {
                return self.fail("execution value byte limit exceeded", span);
            }
            let instance = object
                .as_any()
                .unwrap()
                .downcast_ref::<ScriptObject>()
                .unwrap();
            let variables = &mut self.environments.scope_mut(scope).variables;
            if let Some(stored) = variables.get_slot_mut(slot, name) {
                *stored = value;
            } else {
                // Typed fields can be absent until their first successful write.
                variables.insert(name.into(), value);
            }
            instance.payload.set(payload);
            if measured.has_handles {
                instance.requires_snapshot.set(true);
            }
            return Ok(());
        }
        self.property_result(object.set_property(name, value), span)
    }

    pub(super) fn construct(
        &mut self,
        namespace: Option<&str>,
        name: &str,
        arguments: &[Expr],
        span: Span,
        output: &mut dyn ScriptOutput,
    ) -> Result<Value> {
        self.checkpoint(span)?;
        if self.stack.len().saturating_sub(1)
            >= self.options.recursion_limit.min(MAX_RECURSION_LIMIT)
        {
            return self.fail("maximum construction depth exceeded", span);
        }
        let class = self.class(namespace, name).ok_or_else(|| {
            self.native_error(
                NativeError::new("ClassError", format!("undefined class {name}")),
                span,
            )
        })?;
        if class.is_interface {
            return Err(self.native_error(
                NativeError::new("ClassError", format!("cannot instantiate interface {name}")),
                span,
            ));
        }
        self.ensure_implementation(&class, span)?;
        let arguments = self.evaluate_arguments(arguments, span, output)?;
        self.stack.push(
            StackFrame {
                function: format!("new {name}"),
                span,
                source: self.source.clone(),
            }
            .into(),
        );
        let result = (|| {
            let previous = self.environments.current;
            let previous_source = self.source.clone();
            self.environments.current = class.environment;
            self.source = class.source.clone();
            let defaults = (|| {
                let mut values = Vec::new();
                let mut measured = self
                    .object_size::<true>(&class, FieldValues::PendingSlots(&values), None, span)?
                    .bytes
                    .checked_add(std::mem::size_of::<Value>())
                    .ok_or_else(|| Signal::Error(self.error("object size overflow", span)))?;
                for (owner, field) in class.fields() {
                    self.environments.current = owner.environment;
                    self.source = owner.source.clone();
                    self.checkpoint(span)?;
                    let value = if let Some(default) = &field.default {
                        Some(self.evaluate(default, output)?)
                    } else if field.annotation.is_none() {
                        Some(Value::Null)
                    } else {
                        None
                    };
                    if let Some(value) = value {
                        self.annotation(&field.annotation, &value, span, false)?;
                        let extra = self.field_measure::<false>(&value, 1, span)?.bytes
                            - std::mem::size_of::<Value>();
                        measured = measured.checked_add(extra).ok_or_else(|| {
                            Signal::Error(self.error("object size overflow", span))
                        })?;
                        if self
                            .options
                            .max_value_bytes
                            .is_some_and(|limit| measured > limit)
                        {
                            return self.fail("execution value byte limit exceeded", span);
                        }
                        values.try_reserve(1).map_err(|_| {
                            Signal::Error(self.error("object field allocation failed", span))
                        })?;
                        values.push((field.binding.name.clone(), value));
                    }
                }
                Ok(values)
            })();
            self.environments.current = previous;
            self.source = previous_source;
            let values = defaults?;
            let measured =
                self.object_size::<true>(&class, FieldValues::PendingSlots(&values), None, span)?;
            let payload = measured.bytes;
            let object: Rc<dyn NativeObject> = if class.is_exception() {
                let Value::NativeObject(object) =
                    self.construct_exception(class.clone(), arguments.clone(), span)?
                else {
                    unreachable!()
                };
                let instance = object
                    .as_any()
                    .unwrap()
                    .downcast_ref::<ScriptObject>()
                    .unwrap();
                self.environments
                    .scope_mut(instance.scope)
                    .variables
                    .extend(values);
                instance.payload.set(payload);
                self.validate_value(&Value::NativeObject(object.clone()), 0, span)?;
                object
            } else {
                let variables = crate::bindings::Bindings::Slots(values);
                #[cfg(test)]
                let variables = if self.force_hashed_fields {
                    variables.into_map().into()
                } else {
                    variables
                };
                let previous = self.enter_scope(0, span)?;
                let scope = self.environments.current;
                let root = self.environments.scope_mut(scope);
                root.parent = None;
                root.captured = true;
                root.variables = variables;
                let object: Rc<dyn NativeObject> = Rc::new(ScriptObject {
                    class: class.clone(),
                    scope,
                    payload: Cell::new(payload),
                    requires_snapshot: Cell::new(measured.has_handles),
                    exception: RefCell::new(None),
                });
                self.environments.track_object(&object);
                self.environments.leave(previous);
                object
            };
            if class.method_owner("__construct").is_some() {
                let Value::Callable(constructor) =
                    self.object_property(&object, "__construct", span)?
                else {
                    unreachable!()
                };
                let result = self.call_callable(constructor, arguments, span, output)?;
                if result != Value::Null {
                    return Err(self.native_error(
                        NativeError::type_error("constructors must return null"),
                        span,
                    ));
                }
            } else if !class.is_exception() && !arguments.is_empty() {
                return Err(self.native_error(
                    NativeError::type_error("class without constructor accepts no arguments"),
                    span,
                ));
            }
            Ok(Value::NativeObject(object))
        })();
        self.stack.pop();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherited_direct_methods_protect_lexical_owners_during_arguments_and_unwind() {
        for grouped in [false, true] {
            for fail in [false, true] {
                let mut runtime = Runtime::new();
                // Model a base class declared by an include in a captured local
                // scope, then an exception subclass in a different lexical scope.
                let previous = runtime.environments.enter(0);
                runtime.execute_source(
                    "$secret = 42; class Base extends Error { function read($value: int): int { return $secret + $value; } }",
                    &mut Vec::new(),
                ).unwrap();
                let base = runtime
                    .environments
                    .scope(runtime.environments.current)
                    .classes["Base"]
                    .clone();
                runtime.environments.leave(previous);
                runtime
                    .environments
                    .scope_mut(0)
                    .classes
                    .insert("Base".into(), base);
                runtime.execute_source(
                    "class Child extends Base {} $object = new Child(''); $other = new Child(''); $result = 0;",
                    &mut Vec::new(),
                ).unwrap();
                runtime
                    .register(
                        "prune",
                        NativeFunction::reentrant(
                            NativeSignature::new(vec![], NativeType::Int),
                            move |context, _| {
                                let globals = context.runtime.environments.scope_mut(0);
                                globals.variables.remove("object");
                                globals.variables.remove("other");
                                globals.classes.clear();
                                context.runtime.collect_garbage();
                                if fail {
                                    Err(context
                                        .error(NativeError::new("ProbeError", "argument failed")))
                                } else {
                                    Ok(Value::Int(1))
                                }
                            },
                        ),
                    )
                    .unwrap();
                let source = if grouped {
                    "$result = ($object->read)(($other->read)(prune()));"
                } else {
                    "$result = $object->read($other->read(prune()));"
                };
                let outcome = runtime.execute_source(source, &mut Vec::new());
                if fail {
                    assert!(
                        outcome
                            .unwrap_err()
                            .message
                            .contains("ProbeError: argument failed")
                    );
                    assert_eq!(runtime.get_global("result"), Some(&Value::Int(0)));
                } else {
                    outcome.unwrap();
                    assert_eq!(runtime.get_global("result"), Some(&Value::Int(85)));
                }
                assert!(runtime.environments.temporary_roots.is_empty());
                runtime.collect_garbage();
                assert_eq!(
                    runtime.environments.live, 1,
                    "temporary owners leaked after dispatch"
                );
            }
        }
    }
}
