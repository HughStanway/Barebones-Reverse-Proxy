# Forward Authentication & Single Sign-On (SSO)

`Barebones-Reverse-Proxy` includes a decoupled, trait-based **Forward Authentication Middleware** designed to secure home server services (like Grafana, Speedtest, Sonarr, Radarr, and custom APIs) using central identity providers such as **Authelia** or **Authentik**.

---

## 1. Overview & Architecture

Instead of implementing authentication logic inside individual backend applications, the reverse proxy acts as a centralized authentication gateway:

* **Decoupled `AuthProvider` Trait Interface**: The core proxy engine interacts exclusively with an abstract `AuthProvider` trait in [`src/auth.rs`](file:///Users/hughstanway/Projects/Barebones-Reverse-Proxy/src/auth.rs). This decouples the Rust reverse proxy binary from any specific external authentication software.
* **Route-Level Access Control**: Authentication can be toggled on or off per route using `auth on;` or `auth off;` in expanded `route` blocks.
* **Unified Web & API Key Verification**: Both web browser sessions (cookies) and automated processes (`Authorization: Bearer <token>`, `X-API-Key`) pass through the auth provider sub-request for 100% consistent credential validation.
* **Passkeys, TouchID & FaceID Support**: Integration with Authelia or Authentik enables WebAuthn / FIDO2 authentication, allowing 1-tap FaceID and TouchID login on iOS, iPadOS, macOS, and Android devices.
* **Upstream Identity Header Injection**: Upon successful authentication (`200 OK`), identity headers returned by the auth provider (such as `Remote-User`, `Remote-Groups`, and `Remote-Email`) are automatically injected into the request passed to the backend service.

---

## 2. Configuration Reference

### Security Block Directive

| Directive | Description | Example |
| :--- | :--- | :--- |
| `forward_auth` | The internal HTTP/HTTPS URL of your central authentication provider verification endpoint. | `forward_auth http://localhost:9091/api/verify;` |

### Route Block Directives

| Directive | Description | Options | Default |
| :--- | :--- | :--- | :--- |
| `upstream` | The backend target URL to proxy requests to. | `http://localhost:3002/` | Required in block syntax |
| `auth` | Toggles forward authentication for this route. | `on` / `off` (or `yes`/`no`, `true`/`false`) | `off` |

---

## 3. Configuration Examples

### Complete `proxy.conf` Example

```protobuf
listen 443;
workers 2;
logfile /var/log/proxy.log;

security {
    proxy_protocol on;
    trusted_upstream 10.0.0.1;
    max_tls_failures 5;
    ban_duration 3600;
    rate_limit_rpm 300;

    // Forward Authentication Provider (e.g. Authelia)
    forward_auth http://localhost:9091/api/verify;
}

// Protected Dashboard (Requires Authelia login / TouchID / FaceID / API Key)
route https://grafana.bigiron.dev/ {
    upstream http://localhost:3002/;
    auth on;
    cert /etc/ssl/grafana/cert.pem;
    key /etc/ssl/grafana/key.pem;
}

// Public Dashboard (HTTP route, auth off)
route http://speedtest.bigiron.dev/ {
    upstream http://localhost:4000/;
    auth off;
}
```

---

## 4. Sequence Workflow

```text
Client (Browser / Script)       Reverse Proxy              Authelia Auth Provider        Upstream Service (Grafana)
       |                              |                              |                               |
       |--- HTTPS GET /dashboard ---->|                              |                               |
       |                              |-- Sub-request GET /verify -->|                               |
       |                              |   (Cookie, Auth, X-Forwarded-*)                              |
       |                              |                              |                               |
       |                              |<------- 200 OK --------------|                               |
       |                              |  (Remote-User: hugh)         |                               |
       |                              |                              |                               |
       |                              |----------------------- GET /dashboard ---------------------->|
       |                              |                        (Remote-User: hugh)                   |
       |                              |<---------------------- 200 OK (Dashboard Content) -----------|
       |<-- 200 OK (Content) ---------|                              |                               |
```

If Authelia returns `302 Found` (unauthenticated user), the proxy returns the `Location: https://auth.bigiron.dev/?rd=...` header to the browser, directing the user to log in.

---

## 5. Client Authentication Flow

### A. Web Browsers (Laptops, Phones, Tablets)
1. User navigates to a protected route (e.g., `https://grafana.bigiron.dev/`).
2. If unauthenticated, the user is redirected to the Authelia portal (`https://auth.bigiron.dev/`).
3. The user authenticates using password + **TouchID / FaceID Passkeys** or 2FA/TOTP.
4. An encrypted session cookie is issued for `*.bigiron.dev`.
5. Navigating to any other protected service (e.g. `https://sonarr.bigiron.dev/`) logs the user in **instantly (Single Sign-On)**.

### B. Automated Scripts & APIs (`curl`, Python, Home Assistant)
1. Automated tools include a Bearer Token or API Key in request headers:
   ```bash
   curl -H "Authorization: Bearer <YOUR_API_TOKEN>" https://grafana.bigiron.dev/
   ```
   or
   ```bash
   curl -H "X-API-Key: <YOUR_API_TOKEN>" https://grafana.bigiron.dev/
   ```
2. The proxy passes the `Authorization` / `X-API-Key` header in the `forward_auth` sub-request.
3. Authelia validates the token and returns `200 OK`, allowing the proxy to forward the request to the upstream service without web redirects.

---

## 6. Setting Up the Default Authelia Container

To deploy **Authelia** as the default forward authentication container alongside `Barebones-Reverse-Proxy`, follow this guide.

### Step 1: Create Directory Structure

```bash
mkdir -p /opt/authelia/config
cd /opt/authelia
```

```text
/opt/authelia/
├── docker-compose.yml
└── config/
    ├── configuration.yml
    └── users_database.yml
```

### Step 2: Create `docker-compose.yml`

Create `/opt/authelia/docker-compose.yml`:

```yaml
version: '3.8'

services:
  authelia:
    image: authelia/authelia:latest
    container_name: authelia
    restart: unless-stopped
    ports:
      - "127.0.0.1:9091:9091"
    volumes:
      - ./config:/config
    environment:
      - TZ=UTC
```

### Step 3: Create Authelia Configuration (`config/configuration.yml`)

> [!TIP]
> **Generating Secure Secrets**: You can generate 64-character random secrets for `identity_validation.reset_password.jwt_secret`, `session.secret`, and `storage.encryption_key` using **openssl** or **Authelia CLI**:
> ```bash
> # Option A: Using OpenSSL (Terminal)
> openssl rand -hex 32
> 
> # Option B: Using Authelia Docker Container
> docker run --rm authelia/authelia:latest authelia crypto rand --length 64 --char-set alphanumeric
> ```

Create `/opt/authelia/config/configuration.yml`:

```yaml
server:
  address: 'tcp://0.0.0.0:9091/'

log:
  level: info

identity_validation:
  reset_password:
    jwt_secret: a_random_secure_jwt_secret_key_change_me

default_redirection_url: https://auth.bigiron.dev/

authentication_backend:
  file:
    path: /config/users_database.yml

access_control:
  default_policy: deny
  rules:
    - domain: "*.bigiron.dev"
      policy: one_factor

session:
  name: authelia_session
  secret: a_random_secure_session_secret_key_change_me
  cookies:
    - domain: bigiron.dev
      authelia_url: https://auth.bigiron.dev
  expiration: 3600
  inactivity: 300

regulation:
  max_retries: 3
  find_time: 120
  ban_time: 300

storage:
  encryption_key: a_random_secure_encryption_key_change_me
  local:
    path: /config/db.sqlite

notifier:
  filesystem:
    filename: /config/notification.txt
```

### Step 4: Generate User Password & Configure `users_database.yml`

Generate a secure Argon2id hashed password using the Authelia container:

```bash
docker run --rm authelia/authelia:latest authelia crypto hash generate argon2 --password 'YourStrongPasswordHere'
```

Create `/opt/authelia/config/users_database.yml`:

```yaml
users:
  hugh:
    disabled: false
    displayname: "Hugh Stanway"
    password: "$argon2id$v=19$m=65536,t=3,p=4$..." # Paste generated hash here
    email: hugh@bigiron.dev
    groups:
      - admins
      - dev
```

### Step 5: Start Authelia Container

```bash
docker-compose up -d
```

Verify Authelia is healthy and listening locally on port `9091`:

```bash
curl -i http://127.0.0.1:9091/api/verify
```

### Step 6: Connect Reverse Proxy to Authelia

Update your `proxy.conf` to route authentication verification requests to Authelia:

```protobuf
security {
    forward_auth http://127.0.0.1:9091/api/verify;
}

// Authelia Portal (Public login UI endpoint)
route https://auth.bigiron.dev/ {
    upstream http://localhost:9091/;
    auth off;
    cert /etc/ssl/auth/cert.pem;
    key /etc/ssl/auth/key.pem;
}

// Protected Grafana Service (Enforces Authelia login)
route https://grafana.bigiron.dev/ {
    upstream http://localhost:3002/;
    auth on;
    cert /etc/ssl/grafana/cert.pem;
    key /etc/ssl/grafana/key.pem;
}
```

Reload the reverse proxy configuration cleanly:

```bash
make reload
```

