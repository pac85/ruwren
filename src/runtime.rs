use crate::{UserData, WrenError};
use std::{ffi, os::raw};
use wren_sys::{WrenErrorType, WrenForeignClassMethods, WrenLoadModuleResult, WrenVM};

// Force Wren to use Rust's allocator to allocate memory
// Done because sometimes Wren forces us to allocate memory and give *it* ownership
// Rust might not use the standard allocator, so we move Wren to use *our* allocator
pub extern "C" fn wren_realloc(
    memory: *mut ffi::c_void, new_size: wren_sys::size_t, _user_data: *mut ffi::c_void,
) -> *mut ffi::c_void {
    unsafe {
        if memory.is_null() {
            // If memory == NULL
            if new_size == 0 {
                std::ptr::null_mut()
            } else {
                // allocate new memory
                std::alloc::alloc_zeroed(
                    std::alloc::Layout::from_size_align(new_size as usize, 8).unwrap(),
                ) as *mut _
            }
        } else {
            // Memory is an actual pointer to a location.
            if new_size == 0 {
                std::alloc::dealloc(
                    memory as *mut _,
                    std::alloc::Layout::from_size_align(0, 8).unwrap(),
                );
                std::ptr::null_mut()
            } else {
                std::alloc::realloc(
                    memory as *mut _,
                    std::alloc::Layout::from_size_align(new_size as usize, 8).unwrap(),
                    new_size as usize,
                ) as *mut _
            }
        }
    }
}

pub extern "C" fn wren_error(
    vm: *mut WrenVM, typ: WrenErrorType, module: *const raw::c_char, line: raw::c_int,
    message: *const raw::c_char,
) {
    let conf = unsafe { &mut *(wren_sys::wrenGetUserData(vm) as *mut UserData) };
    match typ {
        wren_sys::WrenErrorType_WREN_ERROR_COMPILE => {
            let module_str = unsafe { ffi::CStr::from_ptr(module) };
            let message_str = unsafe { ffi::CStr::from_ptr(message) };
            conf.error_channel
                .send(WrenError::Compile(
                    module_str.to_string_lossy().to_string(),
                    line as i32,
                    message_str.to_string_lossy().to_string(),
                ))
                .unwrap();
        }
        wren_sys::WrenErrorType_WREN_ERROR_RUNTIME => {
            let message_str = unsafe { ffi::CStr::from_ptr(message) };
            conf.error_channel
                .send(WrenError::Runtime(
                    message_str.to_string_lossy().to_string(),
                ))
                .unwrap();
        }
        wren_sys::WrenErrorType_WREN_ERROR_STACK_TRACE => {
            let module_str = unsafe { ffi::CStr::from_ptr(module) };
            let message_str = unsafe { ffi::CStr::from_ptr(message) };
            conf.error_channel
                .send(WrenError::StackTrace(
                    module_str.to_string_lossy().to_string(),
                    line as i32,
                    message_str.to_string_lossy().to_string(),
                ))
                .unwrap();
        }
        _ => unreachable!(),
    }
}

pub extern "C" fn wren_print(vm: *mut WrenVM, message: *const raw::c_char) {
    let conf = unsafe { &mut *(wren_sys::wrenGetUserData(vm) as *mut UserData) };
    let message_str = unsafe { ffi::CStr::from_ptr(message) };
    conf.printer
        .print(message_str.to_string_lossy().to_string());
}

pub extern "C" fn wren_bind_foreign_method(
    vm: *mut WrenVM, mdl: *const raw::c_char, clss: *const raw::c_char, is_static: bool,
    sgn: *const raw::c_char,
) -> Option<unsafe extern "C" fn(*mut WrenVM)> {
    let conf = unsafe { &mut *(wren_sys::wrenGetUserData(vm) as *mut UserData) };
    let module = unsafe { ffi::CStr::from_ptr(mdl) };
    let class = unsafe { ffi::CStr::from_ptr(clss) };
    let signature = unsafe { ffi::CStr::from_ptr(sgn) };

    let ret = if let Some(ref library) = conf.library {
        if let Some(rc) =
            library.get_foreign_class(module.to_string_lossy(), class.to_string_lossy())
        {
            rc.methods
                .function_pointers
                .iter()
                .find(|mp| {
                    mp.signature.as_wren_string() == signature.to_string_lossy()
                        && mp.is_static == is_static
                })
                .map(|mp| mp.pointer)
        } else {
            None
        }
    } else {
        None
    };

    ret.or_else(|| {
        conf.vm
            .upgrade()
            .and_then(|vm| vm.as_ref().borrow().custom_binders)
            .and_then(|custom_binders| custom_binders.bind_foreign_method_fn)
            .and_then(|f| unsafe { (f)(vm, mdl, clss, is_static, sgn) })
    })
}

pub extern "C" fn wren_bind_foreign_class(
    vm: *mut WrenVM, mdl: *const raw::c_char, clss: *const raw::c_char,
) -> WrenForeignClassMethods {
    let mut fcm = WrenForeignClassMethods {
        allocate: None,
        finalize: None,
    };

    let conf = unsafe { &mut *(wren_sys::wrenGetUserData(vm) as *mut UserData) };
    let module = unsafe { ffi::CStr::from_ptr(mdl) };
    let class = unsafe { ffi::CStr::from_ptr(clss) };

    if let Some(ref library) = conf.library {
        let rc = library.get_foreign_class(module.to_string_lossy(), class.to_string_lossy());
        if let Some(rc) = rc {
            fcm.allocate = Some(rc.construct);
            fcm.finalize = Some(rc.destruct);
        } else if let Some(cfcm) = conf.vm.upgrade().and_then(|vm| vm.as_ref().borrow().custom_binders).and_then(|custom_binders| custom_binders.bind_foreign_class_fn).map(|f| unsafe {(f)(vm, mdl, clss)}) {
            fcm = cfcm;
        }
    }
    fcm
}

pub extern "C" fn wren_load_module(
    vm: *mut WrenVM, name: *const raw::c_char,
) -> WrenLoadModuleResult {
    // The whoooole reason we wrote wren_realloc - to force Wren into Rust's allocation space
    let conf = unsafe { &mut *(wren_sys::wrenGetUserData(vm) as *mut UserData) };
    let module_name = unsafe { ffi::CStr::from_ptr(name) };
    let source = match conf
        .loader
        .load_script(module_name.to_string_lossy().to_string())
    {
        Some(string) => ffi::CString::new(string)
            .unwrap_or_else(|_| {
                panic!(
                    "Failed to convert source to C string for {}",
                    module_name.to_string_lossy()
                )
            })
            .into_raw(),
        None => std::ptr::null_mut(),
    };

    WrenLoadModuleResult {
        source,
        onComplete: None,
        userData: std::ptr::null_mut(),
    }
}

pub extern "C" fn wren_canonicalize(
    _: *mut WrenVM, importer: *const raw::c_char, name: *const raw::c_char,
) -> *const raw::c_char {
    let _importer = unsafe { ffi::CStr::from_ptr(importer) };
    let _name = unsafe { ffi::CStr::from_ptr(name) };
    let _importer = _importer.to_string_lossy();
    let _name = _name.to_string_lossy();

    if let Some('@') = _name.chars().next() {
        let real_name: String = _name.chars().skip(1).collect();
        ffi::CString::new(format!("{}/{}", _importer, real_name))
            .unwrap_or_else(|_| {
                panic!(
                    "Failed to convert name {}/{} to C string",
                    _importer, real_name
                )
            })
            .into_raw() as *const _
    } else {
        name
    }
}
