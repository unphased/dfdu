//! The wheel: it moves the selection, or scrolls the details pane under the pointer or
//! focused.

use super::*;

#[test]
fn wheel_moves_selection() {
    let t0 = Instant::now();
    let mut app = browser(t0);
    let down = mouse(MouseEventKind::ScrollDown, 30, TOP + 4);
    event(&mut app, down, t0).expect("redraw");
    assert_eq!(selected(&app).as_deref(), Some("d1"));
    for _ in 0..5 {
        event(&mut app, down, t0);
    }
    assert_eq!(selected(&app).as_deref(), Some("d6"));
    event(&mut app, mouse(MouseEventKind::ScrollUp, 30, TOP), t0).expect("redraw");
    assert_eq!(selected(&app).as_deref(), Some("d5"));
}

#[test]
fn wheel_over_the_details_pane_leaves_the_list_alone() {
    let t0 = Instant::now();
    let mut app = browser(t0);
    app.key(press(KeyCode::Char('i')), t0);
    render(&mut app, t0, 120, 12);
    let pane = app.areas.details.expect("the pane is shown");
    let list = app.areas.list;
    let down = |r: Rect| mouse(MouseEventKind::ScrollDown, r.x + 1, r.y + 1);
    event(&mut app, down(pane), t0).expect("redraw");
    assert_eq!(selected(&app).as_deref(), Some("d0"));
    event(&mut app, down(list), t0).expect("redraw");
    assert_eq!(selected(&app).as_deref(), Some("d1"));
    // With the pane focused the wheel scrolls it wherever the pointer is.
    app.key(press(KeyCode::Tab), t0);
    render(&mut app, t0, 120, 12);
    event(&mut app, down(list), t0).expect("redraw");
    assert_eq!(selected(&app).as_deref(), Some("d1"));
    // A click on the list still selects with the pane open.
    event(&mut app, left(list.x + 1, TOP + 5), t0).expect("redraw");
    assert_eq!(selected(&app).as_deref(), Some("d5"));
}
