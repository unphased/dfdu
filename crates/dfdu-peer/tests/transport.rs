#![cfg(unix)]
use dfdu_peer::{Peer, Role};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn wait(peer: &Peer, role: Role) {
    let until = Instant::now() + Duration::from_secs(4);
    while peer.status().role != role {
        assert!(Instant::now() < until, "status: {:?}", peer.status());
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn pair_bind_refuse_unbind_and_cleanup() {
    let a = Peer::start(vec![PathBuf::from("/source")]).unwrap();
    let b = Peer::start(vec![]).unwrap();
    let c = Peer::start(vec![]).unwrap();
    assert!(dfdu_peer::list().unwrap().iter().any(|p| p.id == a.id()));
    a.bind(b.id(), "unused").unwrap();
    wait(&a, Role::Master);
    wait(&b, Role::Slave);
    c.bind(b.id(), "unused").unwrap();
    assert!(matches!(
        c.events().recv_timeout(Duration::from_secs(4)).unwrap(),
        dfdu_peer::Event::Error(_)
    ));
    assert_eq!(b.status().role, Role::Slave);
    a.unbind();
    wait(&a, Role::Unbound);
    wait(&b, Role::Unbound);
    b.bind(a.id(), "unused").unwrap();
    wait(&b, Role::Master);
    wait(&a, Role::Slave);
    assert_eq!(
        dfdu_peer::list()
            .unwrap()
            .iter()
            .find(|p| p.id == b.id())
            .unwrap()
            .role,
        Role::Master
    );
    let id = a.id().to_owned();
    drop(a);
    wait(&b, Role::Unbound);
    assert!(!dfdu_peer::list().unwrap().iter().any(|p| p.id == id));
}
