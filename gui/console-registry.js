/**
 * gui/console-registry.js — Single source of truth for all HTML-panel consoles.
 *
 * Maps each lowercase station id (issue #618) to its section id and iframe
 * element id. Prior to #618 the keys were PascalCase Console variant names;
 * the JS layer now works in lowercase station-id space alongside the Rust
 * `StationId` newtype.

 *
 * Used by:
 *  - gui/content-switcher.js  (derives CONSOLE_SECTION + HTML_SECTION_IDS)
 *  - gui/iframe-bridge.js     (look up iframeId for a given console name)
 *  - client.html inline script (via window.CONSOLE_REGISTRY fallback)
 */
export const REGISTRY = Object.freeze({
  captain:    { sectionId: 'captain-ui',    iframeId: 'captain-iframe'    },
  helm:       { sectionId: 'helm-ui',       iframeId: 'helm-iframe'       },
  tactical:   { sectionId: 'weapons-ui',    iframeId: 'weapons-iframe'    },
  repair:     { sectionId: 'repair-ui',     iframeId: 'repair-iframe'     },
  power:      { sectionId: 'power-ui',      iframeId: 'power-iframe'      },
  shields:    { sectionId: 'shields-ui',    iframeId: 'shields-iframe'    },
  sensors:    { sectionId: 'sensors-ui',    iframeId: 'sensors-iframe'    },
  science:    { sectionId: 'science-ui',    iframeId: 'science-iframe'    },
  navigation: { sectionId: 'navigation-ui', iframeId: 'navigation-iframe' },
  comms:      { sectionId: 'comms-ui',      iframeId: 'comms-iframe'      },
});

// Expose for non-module inline scripts (client.html).
if (typeof window !== 'undefined') {
  window.CONSOLE_REGISTRY = REGISTRY;
}
