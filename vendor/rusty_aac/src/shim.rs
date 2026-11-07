//! no_std stand-ins for what the decoder took from `std`.
pub use libm_ext::*;
mod libm_ext {
    pub trait F32Ext { fn sin(self)->f32; fn cos(self)->f32; fn tan(self)->f32; fn sqrt(self)->f32; fn powf(self,e:f32)->f32; fn powi(self,e:i32)->f32; fn exp(self)->f32; fn ln(self)->f32; fn log10(self)->f32; fn log2(self)->f32; fn floor(self)->f32; fn ceil(self)->f32; fn round(self)->f32; fn trunc(self)->f32; fn atan2(self,o:f32)->f32; fn atan(self)->f32; fn cbrt(self)->f32; fn hypot(self,o:f32)->f32; fn sinh(self)->f32; fn cosh(self)->f32; fn tanh(self)->f32; fn exp2(self)->f32; fn mul_add(self,a:f32,b:f32)->f32; fn rem_euclid(self,r:f32)->f32; fn round_ties_even(self)->f32; }
    impl F32Ext for f32 {
        fn round_ties_even(self)->f32{libm::rintf(self)}
        fn sin(self)->f32{libm::sinf(self)} fn cos(self)->f32{libm::cosf(self)} fn tan(self)->f32{libm::tanf(self)} fn sqrt(self)->f32{libm::sqrtf(self)}
        fn powf(self,e:f32)->f32{libm::powf(self,e)} fn powi(self,e:i32)->f32{libm::powf(self,e as f32)} fn exp(self)->f32{libm::expf(self)} fn ln(self)->f32{libm::logf(self)}
        fn log10(self)->f32{libm::log10f(self)} fn log2(self)->f32{libm::log2f(self)} fn floor(self)->f32{libm::floorf(self)} fn ceil(self)->f32{libm::ceilf(self)}
        fn round(self)->f32{libm::roundf(self)} fn trunc(self)->f32{libm::truncf(self)} fn atan2(self,o:f32)->f32{libm::atan2f(self,o)} fn atan(self)->f32{libm::atanf(self)}
        fn cbrt(self)->f32{libm::cbrtf(self)} fn hypot(self,o:f32)->f32{libm::hypotf(self,o)} fn sinh(self)->f32{libm::sinhf(self)} fn cosh(self)->f32{libm::coshf(self)} fn tanh(self)->f32{libm::tanhf(self)}
        fn exp2(self)->f32{libm::exp2f(self)} fn mul_add(self,a:f32,b:f32)->f32{libm::fmaf(self,a,b)} fn rem_euclid(self,r:f32)->f32{let m=libm::fmodf(self,r); if m<0.0 {m+r.abs()} else {m}}
    }
    pub trait F64Ext { fn sin(self)->f64; fn cos(self)->f64; fn tan(self)->f64; fn sqrt(self)->f64; fn powf(self,e:f64)->f64; fn powi(self,e:i32)->f64; fn exp(self)->f64; fn ln(self)->f64; fn log10(self)->f64; fn log2(self)->f64; fn floor(self)->f64; fn ceil(self)->f64; fn round(self)->f64; fn trunc(self)->f64; fn atan2(self,o:f64)->f64; fn atan(self)->f64; fn cbrt(self)->f64; fn hypot(self,o:f64)->f64; fn sinh(self)->f64; fn cosh(self)->f64; fn tanh(self)->f64; fn exp2(self)->f64; fn mul_add(self,a:f64,b:f64)->f64; fn rem_euclid(self,r:f64)->f64; }
    impl F64Ext for f64 {
        fn sin(self)->f64{libm::sin(self)} fn cos(self)->f64{libm::cos(self)} fn tan(self)->f64{libm::tan(self)} fn sqrt(self)->f64{libm::sqrt(self)}
        fn powf(self,e:f64)->f64{libm::pow(self,e)} fn powi(self,e:i32)->f64{libm::pow(self,e as f64)} fn exp(self)->f64{libm::exp(self)} fn ln(self)->f64{libm::log(self)}
        fn log10(self)->f64{libm::log10(self)} fn log2(self)->f64{libm::log2(self)} fn floor(self)->f64{libm::floor(self)} fn ceil(self)->f64{libm::ceil(self)}
        fn round(self)->f64{libm::round(self)} fn trunc(self)->f64{libm::trunc(self)} fn atan2(self,o:f64)->f64{libm::atan2(self,o)} fn atan(self)->f64{libm::atan(self)}
        fn cbrt(self)->f64{libm::cbrt(self)} fn hypot(self,o:f64)->f64{libm::hypot(self,o)} fn sinh(self)->f64{libm::sinh(self)} fn cosh(self)->f64{libm::cosh(self)} fn tanh(self)->f64{libm::tanh(self)}
        fn exp2(self)->f64{libm::exp2(self)} fn mul_add(self,a:f64,b:f64)->f64{libm::fma(self,a,b)} fn rem_euclid(self,r:f64)->f64{let m=libm::fmod(self,r); if m<0.0 {m+r.abs()} else {m}}
    }
}
/// `core::sync::OnceLock` for one thread at a time per cell, made safe by a spin flag.
pub struct OnceLock<T> { state: core::sync::atomic::AtomicU8, value: core::cell::UnsafeCell<Option<T>> }
unsafe impl<T: Send + Sync> Sync for OnceLock<T> {}
impl<T> OnceLock<T> {
    pub const fn new() -> Self { Self { state: core::sync::atomic::AtomicU8::new(0), value: core::cell::UnsafeCell::new(None) } }
    pub fn get(&self) -> Option<&T> { use core::sync::atomic::Ordering::*; if self.state.load(Acquire) == 2 { unsafe { (*self.value.get()).as_ref() } } else { None } }
    pub fn get_or_init(&self, f: impl FnOnce() -> T) -> &T {
        use core::sync::atomic::Ordering::*;
        loop {
            match self.state.compare_exchange(0, 1, Acquire, Acquire) {
                Ok(_) => { unsafe { *self.value.get() = Some(f()); } self.state.store(2, Release); break; }
                Err(2) => break,
                Err(_) => core::hint::spin_loop(),
            }
        }
        unsafe { (*self.value.get()).as_ref().unwrap() }
    }
}
