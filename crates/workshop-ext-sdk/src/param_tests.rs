use super::*;
use core::sync::atomic::AtomicUsize;

static CHANGES: AtomicUsize = AtomicUsize::new(0);
fn changed() {
    CHANGES.fetch_add(1, Ordering::Relaxed);
}

#[test]
fn set_clamps_and_reports_changes_once() {
    static P: Param = Param::new("Level", 3, 1, 9).with_on_change(changed);
    assert_eq!((P.get(), P.is_on()), (3, true));
    assert!(P.set(0));
    assert_eq!(P.get(), 1);
    assert!(!P.set(1), "same value");
    assert!(P.set(100));
    assert_eq!(P.get(), 9);
    assert!(P.reset());
    assert_eq!(P.get(), 3);
    assert_eq!(CHANGES.load(Ordering::Relaxed), 3);
    assert_eq!(P.field(), Field::new("Level", 3, 1, 9));
}

#[test]
fn flags_and_menu_values() {
    static F: Param = Param::flag("Enabled", false);
    assert!(!F.is_on());
    assert!(F.set(7));
    assert_eq!(F.get(), 1);
    assert_eq!(F.field().bits(), 1);
    assert_eq!(Param::from_menu(f32::NAN), None);
    assert_eq!(Param::from_menu(f32::INFINITY), None);
    assert_eq!(Param::from_menu(-3.0), Some(0));
    assert_eq!(Param::from_menu(2.49), Some(2));
    assert_eq!(Param::from_menu(2.5), Some(3));
}

#[test]
fn params_also_build_at_run_time() {
    let p = Param::new("Run", 2, 1, 3).with_on_change(changed);
    assert_eq!((p.get(), p.min, p.max, p.default), (2, 1, 3, 2));
    assert!(Param::flag("F", true).is_on());
}
