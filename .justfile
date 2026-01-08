# print options
default:
    @just --list --unsorted

# install cargo tools
init:
    cargo upgrade --incompatible
    cargo update

# check code
check:
    cargo check
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features
    cargo check --manifest-path examples/cme_mdp3_pcap_dump/Cargo.toml
    cargo fmt --all --manifest-path examples/cme_mdp3_pcap_dump/Cargo.toml -- --check
    cargo clippy --manifest-path examples/cme_mdp3_pcap_dump/Cargo.toml --all-targets --all-features

# automatically fix clippy warnings
fix:
    cargo fmt --all
    cargo clippy --allow-dirty --allow-staged --fix
    cargo fmt --all --manifest-path examples/cme_mdp3_pcap_dump/Cargo.toml
    cargo clippy --manifest-path examples/cme_mdp3_pcap_dump/Cargo.toml --allow-dirty --allow-staged --fix

# build project
build:
    cargo build --all-targets
    cargo build --all-targets --manifest-path examples/cme_mdp3_pcap_dump/Cargo.toml

# execute tests
test:
    cargo test
    cargo test --manifest-path examples/cme_mdp3_pcap_dump/Cargo.toml
