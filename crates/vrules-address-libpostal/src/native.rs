use std::ffi::{CStr, CString, c_char};
use std::ptr;
use std::sync::OnceLock;

use vrules_core::{AddressAnalysis, AddressAnalyzer, AddressComponent};

#[repr(C)]
struct LibpostalNormalizeOptions {
    languages: *mut *mut c_char,
    num_languages: usize,
    address_components: u16,
    latin_ascii: bool,
    transliterate: bool,
    strip_accents: bool,
    decompose: bool,
    lowercase: bool,
    trim_string: bool,
    drop_parentheticals: bool,
    replace_numeric_hyphens: bool,
    delete_numeric_hyphens: bool,
    split_alpha_from_numeric: bool,
    replace_word_hyphens: bool,
    delete_word_hyphens: bool,
    delete_final_periods: bool,
    delete_acronym_periods: bool,
    drop_english_possessives: bool,
    delete_apostrophes: bool,
    expand_numex: bool,
    roman_numerals: bool,
}

#[repr(C)]
struct LibpostalAddressParserOptions {
    language: *mut c_char,
    country: *mut c_char,
}

#[repr(C)]
struct LibpostalAddressParserResponse {
    num_components: usize,
    components: *mut *mut c_char,
    labels: *mut *mut c_char,
}

#[link(name = "postal")]
unsafe extern "C" {
    fn libpostal_setup() -> bool;
    fn libpostal_setup_parser() -> bool;
    fn libpostal_get_default_options() -> LibpostalNormalizeOptions;
    fn libpostal_expand_address(
        input: *mut c_char,
        options: LibpostalNormalizeOptions,
        n: *mut usize,
    ) -> *mut *mut c_char;
    fn libpostal_expansion_array_destroy(expansions: *mut *mut c_char, n: usize);
    fn libpostal_get_address_parser_default_options() -> LibpostalAddressParserOptions;
    fn libpostal_parse_address(
        address: *mut c_char,
        options: LibpostalAddressParserOptions,
    ) -> *mut LibpostalAddressParserResponse;
    fn libpostal_address_parser_response_destroy(response: *mut LibpostalAddressParserResponse);
}

static SETUP: OnceLock<Result<(), String>> = OnceLock::new();

/// Real libpostal analyzer.
#[derive(Debug, Clone, Copy, Default)]
pub struct LibpostalAnalyzer;

impl LibpostalAnalyzer {
    /// Initialize libpostal once for this process.
    ///
    /// # Errors
    /// Returns an error if libpostal's normalization or parser setup fails.
    pub fn new() -> Result<Self, String> {
        setup_once()?;
        Ok(Self)
    }
}

impl AddressAnalyzer for LibpostalAnalyzer {
    fn analyze(&self, text: &str) -> Result<AddressAnalysis, String> {
        setup_once()?;
        let standardized = expand_first(text)?;
        let components = parse_components(text)?;
        Ok(AddressAnalysis {
            input: text.to_string(),
            standardized,
            components,
            confidence: 1.0,
        })
    }
}

fn setup_once() -> Result<(), String> {
    SETUP
        .get_or_init(|| {
            // SAFETY: libpostal setup functions are process-global initializers
            // with no arguments. OnceLock ensures they are called once.
            let ok = unsafe { libpostal_setup() && libpostal_setup_parser() };
            ok.then_some(())
                .ok_or_else(|| "libpostal setup failed".to_string())
        })
        .clone()
}

fn expand_first(text: &str) -> Result<String, String> {
    let input = CString::new(text).map_err(|_| "address contains NUL byte".to_string())?;
    let mut n = 0usize;
    // SAFETY: input is a valid NUL-terminated string for the duration of the
    // call, n is a valid out-pointer, and libpostal owns the returned array.
    let expansions = unsafe {
        libpostal_expand_address(
            input.as_ptr() as *mut c_char,
            libpostal_get_default_options(),
            &mut n,
        )
    };
    if expansions.is_null() || n == 0 {
        return Ok(text.to_string());
    }
    // SAFETY: libpostal returned an array with n entries. The first entry is a
    // NUL-terminated C string owned by libpostal until destroyed below.
    let first = unsafe {
        let ptr = *expansions;
        if ptr.is_null() {
            text.to_string()
        } else {
            CStr::from_ptr(ptr).to_string_lossy().into_owned()
        }
    };
    // SAFETY: expansions/n came from libpostal_expand_address.
    unsafe { libpostal_expansion_array_destroy(expansions, n) };
    Ok(first)
}

fn parse_components(text: &str) -> Result<Vec<AddressComponent>, String> {
    let input = CString::new(text).map_err(|_| "address contains NUL byte".to_string())?;
    // SAFETY: input is valid for the call; libpostal owns the returned response.
    let response = unsafe {
        libpostal_parse_address(
            input.as_ptr() as *mut c_char,
            libpostal_get_address_parser_default_options(),
        )
    };
    if response.is_null() {
        return Ok(Vec::new());
    }
    // SAFETY: response points to a libpostal response until destroyed below.
    let out = unsafe {
        let response_ref = &*response;
        let mut components = Vec::with_capacity(response_ref.num_components);
        for i in 0..response_ref.num_components {
            let label_ptr = *response_ref.labels.add(i);
            let value_ptr = *response_ref.components.add(i);
            if label_ptr == ptr::null_mut() || value_ptr == ptr::null_mut() {
                continue;
            }
            components.push(AddressComponent {
                label: CStr::from_ptr(label_ptr).to_string_lossy().into_owned(),
                value: CStr::from_ptr(value_ptr).to_string_lossy().into_owned(),
            });
        }
        components
    };
    // SAFETY: response came from libpostal_parse_address.
    unsafe { libpostal_address_parser_response_destroy(response) };
    Ok(out)
}
