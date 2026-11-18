-- Preserve the behavior of existing connections. New connections start disabled.
ALTER TABLE sync_connections
    ADD COLUMN inbound_enabled boolean NOT NULL DEFAULT true,
    ADD COLUMN outbound_enabled boolean NOT NULL DEFAULT true;
ALTER TABLE sync_connections
    ALTER COLUMN inbound_enabled SET DEFAULT false,
    ALTER COLUMN outbound_enabled SET DEFAULT false;
