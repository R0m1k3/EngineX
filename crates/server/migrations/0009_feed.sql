-- Étape 7b : fil de notifications, abonnements push web, webhooks entrants.
CREATE TABLE feed (
    id INTEGER PRIMARY KEY,
    at INTEGER NOT NULL,
    source TEXT NOT NULL,                      -- cron, session, verify, download, engine, disk, approval, webhook, agent, system
    level TEXT NOT NULL CHECK (level IN ('info','success','warning','error')),
    title TEXT NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    link TEXT,                                 -- route de l'UI (#sessions/<id>…)
    read INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX feed_unread ON feed(read, id);

CREATE TABLE push_subs (
    id INTEGER PRIMARY KEY,
    endpoint TEXT NOT NULL UNIQUE,
    p256dh TEXT NOT NULL,
    auth TEXT NOT NULL,
    agent TEXT NOT NULL DEFAULT '',            -- navigateur (User-Agent), pour la liste
    created_at INTEGER NOT NULL,
    last_ok INTEGER
);

-- L'adresse (jeton) est le secret : seul son SHA-256 est gardé.
CREATE TABLE webhooks (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    hash BLOB NOT NULL UNIQUE,
    level TEXT NOT NULL DEFAULT 'info' CHECK (level IN ('info','success','warning','error')),
    push INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    last_at INTEGER,
    count INTEGER NOT NULL DEFAULT 0
);
