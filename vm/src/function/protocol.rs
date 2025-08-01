use super::IntoFuncArgs;
use crate::{
    AsObject, PyObject, PyObjectRef, PyPayload, PyResult, TryFromObject, VirtualMachine,
    builtins::{PyDict, PyDictRef, iter::PySequenceIterator},
    convert::ToPyObject,
    identifier,
    object::{Traverse, TraverseFn},
    protocol::{PyIter, PyIterIter, PyMapping, PyMappingMethods},
    types::{AsMapping, GenericMethod},
};
use std::{borrow::Borrow, marker::PhantomData, ops::Deref};

#[derive(Clone, Traverse)]
pub struct ArgCallable {
    obj: PyObjectRef,
    #[pytraverse(skip)]
    call: GenericMethod,
}

impl ArgCallable {
    #[inline(always)]
    pub fn invoke(&self, args: impl IntoFuncArgs, vm: &VirtualMachine) -> PyResult {
        let args = args.into_args(vm);
        (self.call)(&self.obj, args, vm)
    }
}

impl std::fmt::Debug for ArgCallable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArgCallable")
            .field("obj", &self.obj)
            .field("call", &format!("{:08x}", self.call as usize))
            .finish()
    }
}

impl Borrow<PyObject> for ArgCallable {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        &self.obj
    }
}

impl AsRef<PyObject> for ArgCallable {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        &self.obj
    }
}

impl From<ArgCallable> for PyObjectRef {
    #[inline(always)]
    fn from(value: ArgCallable) -> Self {
        value.obj
    }
}

impl TryFromObject for ArgCallable {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let Some(callable) = obj.to_callable() else {
            return Err(
                vm.new_type_error(format!("'{}' object is not callable", obj.class().name()))
            );
        };
        let call = callable.call;
        Ok(Self { obj, call })
    }
}

/// An iterable Python object.
///
/// `ArgIterable` implements `FromArgs` so that a built-in function can accept
/// an object that is required to conform to the Python iterator protocol.
///
/// ArgIterable can optionally perform type checking and conversions on iterated
/// objects using a generic type parameter that implements `TryFromObject`.
pub struct ArgIterable<T = PyObjectRef> {
    iterable: PyObjectRef,
    iter_fn: Option<crate::types::IterFunc>,
    _item: PhantomData<T>,
}

unsafe impl<T: Traverse> Traverse for ArgIterable<T> {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.iterable.traverse(tracer_fn)
    }
}

impl<T> ArgIterable<T> {
    /// Returns an iterator over this sequence of objects.
    ///
    /// This operation may fail if an exception is raised while invoking the
    /// `__iter__` method of the iterable object.
    pub fn iter<'a>(&self, vm: &'a VirtualMachine) -> PyResult<PyIterIter<'a, T>> {
        let iter = PyIter::new(match self.iter_fn {
            Some(f) => f(self.iterable.clone(), vm)?,
            None => PySequenceIterator::new(self.iterable.clone(), vm)?.into_pyobject(vm),
        });
        iter.into_iter(vm)
    }
}

impl<T> TryFromObject for ArgIterable<T>
where
    T: TryFromObject,
{
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let iter_fn = {
            let cls = obj.class();
            let iter_fn = cls.mro_find_map(|x| x.slots.iter.load());
            if iter_fn.is_none() && !cls.has_attr(identifier!(vm, __getitem__)) {
                return Err(vm.new_type_error(format!("'{}' object is not iterable", cls.name())));
            }
            iter_fn
        };
        Ok(Self {
            iterable: obj,
            iter_fn,
            _item: PhantomData,
        })
    }
}

#[derive(Debug, Clone, Traverse)]
pub struct ArgMapping {
    obj: PyObjectRef,
    #[pytraverse(skip)]
    methods: &'static PyMappingMethods,
}

impl ArgMapping {
    #[inline]
    pub const fn with_methods(obj: PyObjectRef, methods: &'static PyMappingMethods) -> Self {
        Self { obj, methods }
    }

    #[inline(always)]
    pub fn from_dict_exact(dict: PyDictRef) -> Self {
        Self {
            obj: dict.into(),
            methods: PyDict::as_mapping(),
        }
    }

    #[inline(always)]
    pub fn mapping(&self) -> PyMapping<'_> {
        PyMapping {
            obj: &self.obj,
            methods: self.methods,
        }
    }
}

impl Borrow<PyObject> for ArgMapping {
    #[inline(always)]
    fn borrow(&self) -> &PyObject {
        &self.obj
    }
}

impl AsRef<PyObject> for ArgMapping {
    #[inline(always)]
    fn as_ref(&self) -> &PyObject {
        &self.obj
    }
}

impl Deref for ArgMapping {
    type Target = PyObject;
    #[inline(always)]
    fn deref(&self) -> &PyObject {
        &self.obj
    }
}

impl From<ArgMapping> for PyObjectRef {
    #[inline(always)]
    fn from(value: ArgMapping) -> Self {
        value.obj
    }
}

impl ToPyObject for ArgMapping {
    #[inline(always)]
    fn to_pyobject(self, _vm: &VirtualMachine) -> PyObjectRef {
        self.obj
    }
}

impl TryFromObject for ArgMapping {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        let mapping = PyMapping::try_protocol(&obj, vm)?;
        let methods = mapping.methods;
        Ok(Self { obj, methods })
    }
}

// this is not strictly related to PySequence protocol.
#[derive(Clone)]
pub struct ArgSequence<T = PyObjectRef>(Vec<T>);

unsafe impl<T: Traverse> Traverse for ArgSequence<T> {
    fn traverse(&self, tracer_fn: &mut TraverseFn<'_>) {
        self.0.traverse(tracer_fn);
    }
}

impl<T> ArgSequence<T> {
    #[inline(always)]
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
    #[inline(always)]
    pub fn as_slice(&self) -> &[T] {
        &self.0
    }
}

impl<T> std::ops::Deref for ArgSequence<T> {
    type Target = [T];
    #[inline(always)]
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<'a, T> IntoIterator for &'a ArgSequence<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl<T> IntoIterator for ArgSequence<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T: TryFromObject> TryFromObject for ArgSequence<T> {
    fn try_from_object(vm: &VirtualMachine, obj: PyObjectRef) -> PyResult<Self> {
        obj.try_to_value(vm).map(Self)
    }
}
