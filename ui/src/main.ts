// Start the vcsstar UI: fonts, tokens, the Zest elements, and the shell.
import "@fontsource-variable/ibm-plex-sans";
import "@fontsource/ibm-plex-mono/400.css";
import "@fontsource/ibm-plex-mono/500.css";
import "@catalystcommunity/design-tokens/src/tokens.css";
import "./styles.css";
import { defineControls } from "@catalystcommunity/controls";
import { defineContainers } from "@catalystcommunity/containers";
import { defineMenus } from "@catalystcommunity/menus";
import { VcsstarApp } from "./app";

defineControls();
defineContainers();
defineMenus();

void new VcsstarApp(document.getElementById("root")!).start();
