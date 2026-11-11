# vcsstar architecture

vcsstar is the place a team works on code: pushes, pull requests, reviews,
statuses, and the API around them. GitHub stays the published copy that the
rest of the world reads, and vcsstar keeps both sides in step in both
directions. When GitHub is down, work on vcsstar continues; when GitHub
comes back, the mirror catches up. This document shows the pieces and how
they fit. The repository guide (`CLAUDE.md`) holds the rules each piece
follows; the plans under `plan/` hold the reasoning.

## The pieces

```mermaid
%%{init: {"theme": "base", "themeVariables": {"fontFamily": "Inter, Helvetica, Arial, sans-serif", "fontSize": "14px", "primaryColor": "#f4f6fb", "primaryBorderColor": "#5b6b8c", "primaryTextColor": "#1f2a44", "lineColor": "#5b6b8c", "clusterBkg": "#fafbfe", "clusterBorder": "#c9d2e3"}, "flowchart": {"curve": "basis", "padding": 14, "nodeSpacing": 40, "rankSpacing": 60}}}%%
flowchart TB
    classDef person fill:#fff4e5,stroke:#c77d12,color:#4a2e00
    classDef pod fill:#e8f1ff,stroke:#2f66b3,color:#0f2a55
    classDef store fill:#eaf7ee,stroke:#2e8b57,color:#0d3b22
    classDef external fill:#f3e8ff,stroke:#7a3fb0,color:#2d1152
    classDef queue fill:#fff0f0,stroke:#b3402f,color:#4a1109

    subgraph People["People and tools"]
        direction LR
        dev["Developer<br/>browser · vcsstar CLI · git"]
        ci["CI and bots<br/>service account tokens"]
    end

    subgraph Instance["vcsstar instance · stateless pods, any number"]
        direction LR
        api["api<br/>CSIL-RPC · web UI<br/>git smart HTTP<br/>webhooks in · metrics"]
        ssh["git-ssh<br/>git over SSH"]
        workers["workers<br/>dispatcher · sweeper · hook<br/>mirror · ingest · repack"]
        cache[("node cache<br/>bare repositories<br/>rebuilt from segments")]
        api --- cache
        ssh --- cache
        workers --- cache
    end

    subgraph Durable["Durable state"]
        direction LR
        pg[("Postgres<br/>refs · pull requests · outbox<br/>identities · mirror state<br/>lane breakers · rate limits")]
        s3[("Object store<br/>history segments<br/>packs · bitmaps")]
        cd[["corndogs<br/>keyed tasks<br/>guarded claims"]]
    end

    subgraph Outside["Outside"]
        direction LR
        gh["GitHub<br/>mirrored repositories<br/>App · webhooks"]
        lk["LinkKeys<br/>sign-in · application keys"]
        prom["Prometheus · OTLP"]
    end

    People -->|"HTTPS · SSH · CSIL-RPC"| Instance
    Instance -->|"rows, refs, outbox"| pg
    Instance -->|"segments"| s3
    Instance <-->|"submit · claim · park"| cd
    Instance <-->|"REST · git push · ls-remote"| gh
    gh -->|"webhooks"| Instance
    Instance <-->|"Rp ops over TCP"| lk
    Instance -.->|"scrape · spans"| prom

    class dev,ci person
    class api,ssh,workers pod
    class pg,s3,cache store
    class gh,lk,prom external
    class cd queue
```

The web UI is plain TypeScript on Zest web components; it speaks CSIL-RPC
to the api pods and nothing else. Every pod runs the roles its deployment names. The pods hold no state of
their own: sessions, tokens, refs, pull requests, the outbox, mirror state,
the GitHub lane breakers, the rate table, and the pass clocks live in
Postgres; pending work lives in corndogs; history lives in the object store.
A pod's repository cache is a disposable projection that it rebuilds in the
background after a restart. Any number of pods can therefore run every role.

## A push

A push is one transaction. The hook is the transaction point, Postgres holds
the refs with compare-and-swap, and the pack becomes an immutable history
segment before the push is acknowledged.

```mermaid
%%{init: {"theme": "base", "themeVariables": {"fontFamily": "Inter, Helvetica, Arial, sans-serif", "fontSize": "14px", "primaryColor": "#f4f6fb", "primaryBorderColor": "#5b6b8c", "primaryTextColor": "#1f2a44", "lineColor": "#5b6b8c", "actorBkg": "#e8f1ff", "actorBorder": "#2f66b3", "signalColor": "#1f2a44", "noteBkgColor": "#fff4e5", "noteBorderColor": "#c77d12"}}}%%
sequenceDiagram
    autonumber
    participant G as git client
    participant S as api / git-ssh pod
    participant H as pre-receive hook
    participant P as Postgres
    participant O as Object store
    participant Q as corndogs

    G->>S: git push (HTTPS token or SSH key)
    S->>P: load refs, hydrate cache (segments the node lacks)
    S->>S: git receive-pack with the pack in quarantine
    S->>H: ref updates over the hook socket
    H->>P: begin: protections, signatures, CAS on each ref
    H->>O: upload the quarantined pack as a segment
    H->>P: manifest row, outbox events, pull request updates, commit
    H-->>S: accept (or remote: rejection lines)
    S-->>G: push result
    Note over P,Q: dispatcher reads the outbox
    P->>Q: one keyed task per hook delivery and per mirror object key
```

## Keeping GitHub in step

The dispatcher turns outbox events into at most one pending mirror action
per object key (a ref, a pull request, a comment, a status). Workers claim
tasks from corndogs, read the desired state from Postgres at that moment,
and converge GitHub to it. Inbound, every GitHub webhook is captured first
and processed later; a pull-based pass asks GitHub for what webhooks missed.

```mermaid
%%{init: {"theme": "base", "themeVariables": {"fontFamily": "Inter, Helvetica, Arial, sans-serif", "fontSize": "14px", "primaryColor": "#f4f6fb", "primaryBorderColor": "#5b6b8c", "primaryTextColor": "#1f2a44", "lineColor": "#5b6b8c", "clusterBkg": "#fafbfe", "clusterBorder": "#c9d2e3"}, "flowchart": {"curve": "basis"}}}%%
flowchart TB
    classDef pg fill:#eaf7ee,stroke:#2e8b57,color:#0d3b22
    classDef work fill:#e8f1ff,stroke:#2f66b3,color:#0f2a55
    classDef gh fill:#f3e8ff,stroke:#7a3fb0,color:#2d1152
    classDef queue fill:#fff0f0,stroke:#b3402f,color:#4a1109
    classDef guard fill:#fff4e5,stroke:#c77d12,color:#4a2e00

    subgraph Out["vcsstar to GitHub"]
        direction LR
        ev[("outbox events<br/>origin stamped")]:::pg
        disp["dispatcher<br/>skips origin=github<br/>and shadow actors"]:::work
        act[("mirror_actions<br/>one pending per key<br/>newest state wins")]:::pg
        q1[["mirror:org/repo"]]:::queue
        mw["mirror-worker<br/>token: user link → App → credential"]:::work
        ev --> disp --> act --> q1 --> mw
    end

    gh["GitHub<br/>repository · pull requests · statuses"]:::gh
    mw -->|"git push --force-with-lease<br/>REST create / patch"| gh

    subgraph In["GitHub to vcsstar"]
        direction LR
        hook["POST /hooks/github<br/>verify signature · capture · 202"]:::work
        q2[["ingest:github"]]:::queue
        ing["ingest-worker<br/>applies under origin=github"]:::work
        rows[("pull requests · reviews<br/>comments · statuses<br/>github ids · updated_at")]:::pg
        hook --> q2 --> ing --> rows
    end
    gh -->|"webhooks (App)"| hook

    subgraph Floor["Floor under the webhooks"]
        direction LR
        rec["reconcile pass<br/>ls-remote refs every 15 min"]:::work
        pull["pull pass<br/>pull requests by updated_at<br/>down to the watermark"]:::work
    end
    gh -.->|"state, not events"| rec
    gh -.-> pull
    rec --> act
    pull --> rows

    breaker{{"github_health<br/>lane breakers api · git<br/>per-token rate limits<br/>shared in Postgres"}}:::guard
    breaker -.->|"hold while open<br/>probe · recover · re-check all"| mw
    breaker -.-> rec
    breaker -.-> pull
```

What makes this safe to repeat: every row remembers its GitHub id and
GitHub's `updated_at`, bodies vcsstar posts carry a marker, a snapshot older
than the applied one is ignored, and nothing written under origin `github`
or by a shadow actor is ever sent back.

## When GitHub is down

```mermaid
%%{init: {"theme": "base", "themeVariables": {"fontFamily": "Inter, Helvetica, Arial, sans-serif", "fontSize": "14px", "primaryColor": "#f4f6fb", "primaryBorderColor": "#5b6b8c", "primaryTextColor": "#1f2a44", "lineColor": "#5b6b8c"}}}%%
stateDiagram-v2
    direction LR
    [*] --> Closed
    Closed --> Open: 3 transient failures in a row<br/>(5xx, network, rate limit)
    Open --> HalfOpen: hold ends<br/>(30 s, doubling to 10 min)
    HalfOpen --> Closed: the one probe succeeds<br/>recovered_at set, every repository re-checked
    HalfOpen --> Open: the probe fails<br/>hold doubles
    Closed --> Closed: 4xx answers<br/>GitHub is up, it said no

    note right of Open
        Actions are held, not attempted.
        Parked tasks wait in corndogs.
        Work on vcsstar continues.
    end note
```

Work made on vcsstar during an outage is a bounded set of pending actions,
not a log: a day of pushes to one branch is one action carrying the newest
target. Work made on GitHub during an outage is found by the pull pass when
the lane recovers. Rate limits are respected per token: below a reserve the
token waits for its window, and a `Retry-After` is honoured.

## Identity

```mermaid
%%{init: {"theme": "base", "themeVariables": {"fontFamily": "Inter, Helvetica, Arial, sans-serif", "fontSize": "14px", "primaryColor": "#f4f6fb", "primaryBorderColor": "#5b6b8c", "primaryTextColor": "#1f2a44", "lineColor": "#5b6b8c", "clusterBkg": "#fafbfe", "clusterBorder": "#c9d2e3"}, "flowchart": {"curve": "basis"}}}%%
flowchart LR
    classDef person fill:#fff4e5,stroke:#c77d12,color:#4a2e00
    classDef pod fill:#e8f1ff,stroke:#2f66b3,color:#0f2a55
    classDef external fill:#f3e8ff,stroke:#7a3fb0,color:#2d1152
    classDef store fill:#eaf7ee,stroke:#2e8b57,color:#0d3b22

    person["Person<br/>user_id@domain"]:::person
    svc["Service account<br/>user_id@svc.instance"]:::person
    lk["LinkKeys home domain<br/>sign-in · signed claims<br/>application keys"]:::external
    api["vcsstar api"]:::pod
    gh["GitHub account<br/>linked through the App<br/>or a personal token"]:::external
    db[("identities · sessions · tokens<br/>github_links · ssh_keys")]:::store

    person -->|"browser or device login"| lk --> api
    person -->|"API token · SSH key<br/>attested by the home domain"| api
    svc -->|"API token only<br/>never signs in"| api
    api --> db
    person -.->|"Connect GitHub"| gh
    gh -.->|"acts as the person on GitHub<br/>GitHub activity attributed back"| api
```

Identity keys are `user_id@domain` and never a handle. Standing consent at
the home domain is per application: vcsstar signs people in as the
application `vcsstar` of its LinkKeys account, the same application its
instance keys and act-as grants use. A linked GitHub
account makes vcsstar act on GitHub as that person; an unlinked actor's
work goes through the instance's GitHub App, then the organization's
credential. GitHub accounts without a link appear as shadow identities.

## Where to read more

| Topic | Where |
| --- | --- |
| Rules every piece follows, local development, roles, configuration | `CLAUDE.md` |
| The contract every client and the CLI are generated from | `csil/vcsstar.csil` |
| Chart and instance overlays | `helm/vcsstar`, `deploy/values-example.yaml` |
| Future thinking on CI scale (bundle-URI, memory cache workers) | `plan/15-future-ci-scale.md` (local plans, not in the repository) |
