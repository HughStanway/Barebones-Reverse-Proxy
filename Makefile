# ---- Config ----
CARGO := cargo
SERVICE_NAME ?= barebones-reverse-proxy.service
SYSTEMCTL ?= sudo systemctl

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

# ---- Deployment Cycle ----
.PHONY: deploy
deploy:
	chmod +x scripts/deploy.sh
	./scripts/deploy.sh

.PHONY: reload
reload:
	@$(SYSTEMCTL) reload "$(SERVICE_NAME)"
	@echo "Reloaded systemd service $(SERVICE_NAME)"

.PHONY: down
down:
	@$(SYSTEMCTL) stop "$(SERVICE_NAME)"
	@echo "Stopped systemd service $(SERVICE_NAME)"
