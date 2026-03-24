# ---- Config ----
CARGO := cargo

# ---- Default ----
.PHONY: all
all: build

# ---- Development ----
.PHONY: build
build:
	$(CARGO) build

.PHONY: run
run:
	$(CARGO) run

.PHONY: check
check:
	$(CARGO) check

# ---- Formatting & Linting ----
.PHONY: fmt
fmt:
	$(CARGO) fmt

.PHONY: fmt-check
fmt-check:
	$(CARGO) fmt -- --check

.PHONY: lint
lint:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

# ---- Release ----
.PHONY: release
release:
	$(CARGO) build --release

# ---- Testing ----
.PHONY: test
test:
	$(CARGO) test

# ---- Clean ----
.PHONY: clean
clean:
	$(CARGO) clean
