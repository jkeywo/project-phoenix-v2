/**
 * gui/console-registry.js — Single source of truth for all HTML-panel consoles.
 *
 * Maps each PascalCase Console enum variant (those backed by an HTML iframe in
 * client.html) to its section id and iframe element id. Bevy-rendered consoles
 * (Navigation) are absent — they have no HTML panel.
 *
 * Used by:
 *  - gui/content-switcher.js  (derives CONSOLE_SECTION + HTML_SECTION_IDS)
 *  - gui/iframe-bridge.js     (look up iframeId for a given console name)
 *  - client.html inline script (via window.CONSOLE_REGISTRY fallback)
 */
export const REGISTRY = Object.freeze({
  CaptainChair: { sectionId: 'captain-ui',  iframeId: 'captain-iframe'  },
  Helm:         { sectionId: 'helm-ui',     iframeId: 'helm-iframe'     },
  Tactical:     { sectionId: 'weapons-ui',  iframeId: 'weapons-iframe'  },
  Repair:       { sectionId: 'repair-ui',   iframeId: 'repair-iframe'   },
  Power:        { sectionId: 'power-ui',    iframeId: 'power-iframe'    },
  Shields:      { sectionId: 'shields-ui',  iframeId: 'shields-iframe'  },
  Sensors:      { sectionId: 'sensors-ui',  iframeId: 'sensors-iframe'  },
  Comms:        { sectionId: 'comms-ui',    iframeId: 'comms-iframe'    },
});

// Expose for non-module inline scripts (client.html).
if (typeof window !== 'undefined') {
  window.CONSOLE_REGISTRY = REGISTRY;
}
