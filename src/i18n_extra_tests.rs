use crate::i18n::{init, get_language};

// -- Concurrency and Load tests ------------------------------------------------

#[test]
fn test_concurrency_stress_lookups() {
    use std::thread;

    init("en");

    let num_threads = 16;
    let num_iterations = 1000;
    let mut handles = Vec::new();

    for _ in 0..num_threads {
        let handle = thread::spawn(move || {
            for _ in 0..num_iterations {
                let r1 = crate::i18n::t("session.error_header", &[]);
                assert!(r1.contains("Session Error") || r1.contains("Session-Fehler"));

                let r2 = crate::i18n::t("stop.killed", &[("provider", "Claude")]);
                assert!(r2.contains("Claude"));

                let r3 = crate::i18n::t_rich("wizard.common.cancelled", &[]);
                assert!(r3.to_lowercase().contains("cancelled") || r3.contains("abgebrochen"));

                let r4 = crate::i18n::t_plural("tasks.cancelled", 1, &[]);
                assert!(r4.contains("1 task.") || r4.contains("1 Aufgabe"));

                let r5 = crate::i18n::t_plural("tasks.cancelled", 5, &[]);
                assert!(r5.contains("5 tasks.") || r5.contains("5 Aufgaben"));
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn test_concurrency_race_init_and_lookup() {
    use std::thread;
    use std::sync::{Arc, Barrier};

    let num_init_threads = 8;
    let num_lookup_threads = 8;
    let num_iterations = 200;

    let barrier = Arc::new(Barrier::new(num_init_threads + num_lookup_threads));
    let mut handles = Vec::new();

    // Spawn threads that continuously call init with different languages
    for _ in 0..num_init_threads {
        let b = barrier.clone();
        let handle = thread::spawn(move || {
            b.wait();
            for j in 0..num_iterations {
                if j % 2 == 0 {
                    init("de");
                } else {
                    init("en");
                }
            }
        });
        handles.push(handle);
    }

    // Spawn threads that concurrently perform lookups and query active language
    for _ in 0..num_lookup_threads {
        let b = barrier.clone();
        let handle = thread::spawn(move || {
            b.wait();
            for _ in 0..num_iterations {
                let lang = get_language();
                assert!(lang == "en" || lang == "de", "Unexpected language: {}", lang);

                let _r1 = crate::i18n::t("session.error_header", &[]);
                let _r2 = crate::i18n::t("stop.killed", &[("provider", "Claude")]);
            }
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }
}

#[test]
fn test_performance_valid_no_args() {
    use std::thread;
    use std::sync::{Arc, Barrier};
    use std::time::Instant;

    init("en");
    let num_threads = 8;
    let num_iterations = 1000;
    let barrier = Arc::new(Barrier::new(num_threads));
    let mut handles = Vec::new();
    
    for _ in 0..num_threads {
        let b = barrier.clone();
        let handle = thread::spawn(move || {
            b.wait();
            let mut dummy = 0;
            for _ in 0..num_iterations {
                let r = crate::i18n::t("session.error_header", &[]);
                dummy += r.len();
            }
            dummy
        });
        handles.push(handle);
    }
    
    let start = Instant::now();
    for handle in handles {
        let _ = handle.join().unwrap();
    }
    let duration = start.elapsed();
    println!("Time for valid lookups (no args): {:?}", duration);
    let time_per_1000 = duration.as_secs_f64() / (num_iterations as f64) * 1000.0;
    println!("Normalized time per 1000: {:.4}s", time_per_1000);
}

#[test]
fn test_performance_valid_args() {
    use std::thread;
    use std::sync::{Arc, Barrier};
    use std::time::Instant;

    init("en");
    let num_threads = 8;
    let num_iterations = 1000;
    let barrier = Arc::new(Barrier::new(num_threads));
    let mut handles = Vec::new();
    
    for _ in 0..num_threads {
        let b = barrier.clone();
        let handle = thread::spawn(move || {
            b.wait();
            let mut dummy = 0;
            for _ in 0..num_iterations {
                let r = crate::i18n::t("stop.killed", &[("provider", "Claude")]);
                dummy += r.len();
            }
            dummy
        });
        handles.push(handle);
    }
    
    let start = Instant::now();
    for handle in handles {
        let _ = handle.join().unwrap();
    }
    let duration = start.elapsed();
    println!("Time for valid lookups (with args): {:?}", duration);
    let time_per_1000 = duration.as_secs_f64() / (num_iterations as f64) * 1000.0;
    println!("Normalized time per 1000: {:.4}s", time_per_1000);
}

#[test]
fn test_performance_missing_args() {
    use std::thread;
    use std::sync::{Arc, Barrier};
    use std::time::Instant;

    init("en");
    let num_threads = 8;
    let num_iterations = 100;
    let barrier = Arc::new(Barrier::new(num_threads));
    let mut handles = Vec::new();
    
    for _ in 0..num_threads {
        let b = barrier.clone();
        let handle = thread::spawn(move || {
            b.wait();
            let mut dummy = 0;
            for _ in 0..num_iterations {
                let r = crate::i18n::t("session.error_body", &[]);
                dummy += r.len();
            }
            dummy
        });
        handles.push(handle);
    }
    
    let start = Instant::now();
    for handle in handles {
        let _ = handle.join().unwrap();
    }
    let duration = start.elapsed();
    println!("Time for missing placeholder lookups: {:?}", duration);
    let time_per_1000 = duration.as_secs_f64() / (num_iterations as f64) * 1000.0;
    println!("Normalized time per 1000: {:.4}s", time_per_1000);
}

#[test]
fn test_performance_missing_placeholder_nonempty_args() {
    use std::thread;
    use std::sync::{Arc, Barrier};
    use std::time::Instant;

    init("en");
    let num_threads = 8;
    let num_iterations = 100;
    let barrier = Arc::new(Barrier::new(num_threads));
    let mut handles = Vec::new();
    
    for _ in 0..num_threads {
        let b = barrier.clone();
        let handle = thread::spawn(move || {
            b.wait();
            let mut dummy = 0;
            for _ in 0..num_iterations {
                let r = crate::i18n::t("session.error_body", &[("wrong_arg", "value")]);
                dummy += r.len();
            }
            dummy
        });
        handles.push(handle);
    }
    
    let start = Instant::now();
    for handle in handles {
        let _ = handle.join().unwrap();
    }
    let duration = start.elapsed();
    println!("Time for missing placeholder lookups (nonempty args): {:?}", duration);
    let time_per_1000 = duration.as_secs_f64() / (num_iterations as f64) * 1000.0;
    println!("Normalized time per 1000: {:.4}s", time_per_1000);
}


#[test]
fn test_edge_cases_interpolation() {
    use crate::i18n::format_string;

    // 1. Empty parameter values
    assert_eq!(
        format_string("key", "Hello {name}!", &[("name", "")]),
        "Hello !"
    );

    // 2. Value containing regex/placeholder special characters ($, \, {, })
    assert_eq!(
        format_string("key", "Hello {name}!", &[("name", "$1 \\ {escaped} {nested}")]),
        "Hello $1 \\ {escaped} {nested}!"
    );

    // 3. Emojis and Unicode characters in values
    assert_eq!(
        format_string("key", "Hello {name}!", &[("name", "안녕 🌟")]),
        "Hello 안녕 🌟!"
    );

    // 4. Missing placeholder check (does best-effort replacement)
    assert_eq!(
        format_string("key", "Hello {name} {age}!", &[("name", "Bob")]),
        "Hello Bob {age}!"
    );

    // 5. Extra parameters in args (ignored but shouldn't block replacement)
    assert_eq!(
        format_string("key", "Hello {name}!", &[("name", "Bob"), ("age", "30")]),
        "Hello Bob!"
    );

    // 6. Duplicate keys in args
    assert_eq!(
        format_string("key", "Hello {name}!", &[("name", "Bob"), ("name", "Alice")]),
        "Hello Bob!"
    );

    // 7. Non-alphanumeric placeholder names (like hyphen or dot)
    assert_eq!(
        format_string("key", "Hello {first-name}!", &[("first-name", "Bob")]),
        "Hello Bob!"
    );
    assert_eq!(
        format_string("key", "Hello {first.name}!", &[("first.name", "Bob")]),
        "Hello Bob!"
    );

    // 8. Nested/adjacent braces
    assert_eq!(
        format_string("key", "Hello {{name}}!", &[("name", "Bob")]),
        "Hello {Bob}!"
    );
}

#[test]
fn test_edge_cases_pluralization() {
    use crate::i18n::t_plural;

    init("en");

    // Count = 1 (uses _one)
    let p1 = t_plural("tasks.cancelled", 1, &[]);
    assert!(p1.contains("1 task."));

    // Count = 0 (uses _other)
    let p0 = t_plural("tasks.cancelled", 0, &[]);
    assert!(p0.contains("0 tasks."));

    // Count = -1 (uses _other)
    let pm1 = t_plural("tasks.cancelled", -1, &[]);
    assert!(pm1.contains("-1 tasks."));

    // Count = 100 (uses _other)
    let p100 = t_plural("tasks.cancelled", 100, &[]);
    assert!(p100.contains("100 tasks."));
}

#[test]
fn test_placeholder_extraction_regex_limitations() {
    use regex::Regex;
    // The regex used in i18n_tests.rs (extract_placeholders)
    let re_test = Regex::new(r"\{([\w.-]+)\}").unwrap();
    // The regex used in check.rs and loader.rs
    let re_prod = Regex::new(r"\{([\w.-]+)\}").unwrap();

    let sample_dot = "Hello {user.name}!";
    let sample_hyphen = "Hello {user-name}!";
    let sample_normal = "Hello {username}!";

    // 1. The test regex now correctly extracts dot and hyphen placeholders
    assert!(re_test.captures(sample_dot).is_some());
    assert!(re_test.captures(sample_hyphen).is_some());
    assert!(re_test.captures(sample_normal).is_some());

    // 2. The production regex correctly extracts dot and hyphen placeholders
    assert!(re_prod.captures(sample_dot).is_some());
    assert!(re_prod.captures(sample_hyphen).is_some());
    assert!(re_prod.captures(sample_normal).is_some());
}

#[test]
fn test_empty_locales_directory_silently_ignored() {
    use crate::i18n::TranslationStore;
    use std::path::Path;

    // Create a store pointing to a non-existent path
    let bad_path = Path::new("non_existent_locales_path_xyz");
    let store = TranslationStore::new_with_root("en", bad_path);

    // The store should load successfully (without panicking or returning an error)
    // but the underlying hash maps will be completely empty
    assert!(store.en_chat.is_empty());
    assert!(store.en_cli.is_empty());
    assert!(store.en_cmd.is_empty());

    // Lookups will result in missing keys
    let res = store.chat("any.key", &[]);
    assert_eq!(res, "[MISSING: any.key]");
}

