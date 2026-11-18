-- Older versions have one pause switch. Do not enable a disabled direction.
UPDATE sync_connections SET paused = true
WHERE NOT inbound_enabled OR NOT outbound_enabled;

ALTER TABLE sync_connections
    DROP COLUMN inbound_enabled,
    DROP COLUMN outbound_enabled;
