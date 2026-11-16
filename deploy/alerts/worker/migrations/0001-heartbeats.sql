-- 只登记两个固定节点；首次检测从建库时开始给予 180 秒宽限。
CREATE TABLE heartbeats (
    node TEXT PRIMARY KEY CHECK (node IN ('n108', 'n169')),
    last_seen INTEGER NOT NULL,
    last_sent INTEGER NULL,
    lease_until INTEGER NOT NULL DEFAULT 0
);

INSERT INTO heartbeats (node, last_seen) VALUES
    ('n108', unixepoch()),
    ('n169', unixepoch());
