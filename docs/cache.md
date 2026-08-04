# LRU Static Asset Cache Documentation

`Barebones-Reverse-Proxy` includes an in-memory **Least Recently Used (LRU) Static Asset Cache** designed to serve static files (`.js`, `.css`, images, fonts, WASM, SVG) directly from RAM in **`< 0.3ms`**, bypassing upstream backends and reducing server load.

---

## 1. Architectural Overview

```text
                                  ┌───────────────────────────────┐
                                  │      LRU CACHE IN MEMORY      │
                                  │  (Concurrent Capacity Limit)  │
                                  └──────────────┬────────────────┘
                                                 │
                                           HIT?  │ (0.3ms Direct RAM response)
                                           ┌─────┴─────┐
                                           │           │
                                       YES │           │ NO (MISS)
                                           ▼           ▼
Browser ──► Barebones-Reverse-Proxy ──► Return RAM  ──► Fetch Upstream ──► Store LRU ──► Return
```

### Key Technical Highlights
* **Zero-Copy Memory Design**: Leverages reference-counted `hyper::body::Bytes` buffers (`Arc` internally) for cached bodies, serving static assets with zero heap re-allocations or memory copies.
* **Thread Safety**: Low-contention `RwLock` synchronization allows parallel worker threads to perform concurrent read lookups simultaneously.
* **Byte-Accurate Capacity Budgeting**: Rather than relying purely on item count, the cache tracks exact byte usage (`current_memory_bytes`). When capacity limit is reached, it automatically evicts the Least Recently Used items.
* **Dynamic Configuration Reloading**: Dynamic configuration reloads (`make reload`) preserve un-expired cache entries across route updates.

---

## 2. Configuration Reference (`proxy.conf`)

### Global Cache Block

Add a `cache { ... }` block to `proxy.conf`:

```protobuf
cache {
    enabled on;             // Enable or disable static asset caching (default: off)
    max_capacity_mb 64;     // Total RAM capacity allocated for cache (default: 64MB)
    max_file_size_mb 2;     // Maximum size of an individual cached file (default: 2MB)
    default_ttl_sec 300;    // Default Time-To-Live in seconds if upstream omits max-age (default: 300s / 5 min)
}
```

### Directives Table

| Directive | Type | Default | Description |
| :--- | :--- | :--- | :--- |
| `enabled` | `on` / `off` | `off` | Master toggle for the LRU static asset cache. |
| `max_capacity_mb` | Size string / int | `64M` | Total RAM memory budget for the cache (e.g. `64M`, `128M`, `1G`). |
| `max_file_size_mb` | Size string / int | `2M` | File size ceiling for an individual asset to be cached. |
| `default_ttl_sec` | Integer | `300` | Fallback cache expiration time in seconds. |

### Per-Route Cache Overrides

By default, **all routes inherit caching (`cache on`)** as long as the global `cache.enabled` master toggle is `on`. You can explicitly override caching per route:

```protobuf
route https://dashboard.home/ {
    upstream http://localhost:3001/;
    auth on;
    cache on;   // Explicitly enable (this is also the default if omitted)
}

route https://api.home/ {
    upstream http://localhost:4000/;
    cache off;  // Explicitly disable caching for dynamic API route
}
```

---

## 3. Caching Ruleset & Decision Tree

The proxy evaluates incoming requests and upstream responses against the following ruleset:

### A. Cache Lookup Eligibility (Pre-Flight)
A request is checked against the LRU cache if **ALL** conditions are met:
1. Master `cache.enabled` is `on` (or route specifies `cache on`).
2. HTTP Method is `GET` or `HEAD`.
3. Request URL matches an active route.

### B. Asset Identification Rules
An upstream response is classified as a cacheable static asset if:
* **File Extension**: Path ends with `.css`, `.js`, `.png`, `.jpg`, `.jpeg`, `.svg`, `.ico`, `.woff2`, `.woff`, `.ttf`, `.webp`, `.html`, `.json`, or `.wasm`.
* **Content-Type Header**: Header contains `text/css`, `application/javascript`, `text/javascript`, `image/`, `font/`, or `application/wasm`.

### C. Upstream Response Eligibility
An asset is saved to cache only if:
1. Upstream HTTP status code is **`200 OK`**.
2. Upstream `Cache-Control` header does **NOT** contain `no-store` or `private`.
3. Response body size is `<= max_file_size_mb`.

---

## 4. Response Headers (`X-Proxy-Cache`)

The proxy automatically injects an `X-Proxy-Cache` HTTP header into every response for full observability:

| Header Value | Description |
| :--- | :--- |
| **`X-Proxy-Cache: HIT`** | Served directly from in-memory RAM cache (`< 0.3ms`). |
| **`X-Proxy-Cache: MISS`** | Fetched from upstream, cached in memory, and delivered to client. |
| **`X-Proxy-Cache: BYPASS`** | Request or response skipped caching (uncacheable endpoint, non-GET method, `Cache-Control: no-store`, or file size limit exceeded). |

---

## 5. Structured Telemetry Logging

Every cache lookup, hit, miss, insertion, and bypass emits structured log attributes into the logfile:

### Cache HIT Log Line
```text
event="cache_hit" client_ip="192.168.1.50" host="dashboard.home" path="/static/app.js" size_bytes=24500 total_cache_bytes=1048576 total_cache_entries=15 max_capacity_bytes=67108864
```

### Cache MISS & Inserted Log Line
```text
event="cache_miss" client_ip="192.168.1.50" host="dashboard.home" path="/static/style.css" status=200 cache_action="inserted" bytes_inserted=12400 bytes_evicted=0 items_evicted=0 total_cache_bytes=1060976 total_cache_entries=16 max_capacity_bytes=67108864
```

### Cache MISS & Bypassed Log Line
```text
event="cache_miss" client_ip="192.168.1.50" host="dashboard.home" path="/api/data" status=200 cache_action="bypassed" reason="non_static_asset" total_cache_bytes=1060976 total_cache_entries=16 max_capacity_bytes=67108864
```

### Log Field Reference

* **`total_cache_bytes`**: Current total RAM byte footprint of all stored assets.
* **`total_cache_entries`**: Total number of cached items.
* **`max_capacity_bytes`**: Maximum configured cache RAM capacity ceiling.
* **`bytes_inserted`**: Bytes written to cache for the current asset.
* **`bytes_evicted` & `items_evicted`**: Quantity of bytes and items evicted during LRU capacity maintenance.
* **`reason`**: Explanation when caching is bypassed (`non_static_asset`, `cache_control_no_store`, `exceeds_max_file_size`, `non_200_status`, `cache_disabled`).
