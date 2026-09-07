import { test, expect } from '@playwright/test';

// Populated phone composition from #1393, including the space taken by the
// Station Bar. Empty custom elements conceal the overflowing three-rail stack.
for (const blocked of [false, true]) {
for (const [width, height] of [[375, 812], [390, 844], [375, 760], [390, 792], [716, 375], [812, 375], [1280, 720]]) {
  test(`Cruiser Tactical populated fit ${width}x${height} ${blocked ? 'blocked / four shields' : 'normal'}`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await page.goto('/gui/cruiser/tactical.html');
    await page.waitForFunction(() => customElements.get('ph-target-lock-card'));
    await page.evaluate(async (blocked) => {
      const { renderStation } = await import('/gui/cruiser/tactical.console.js');
      renderStation({ system_families: { weapons: 'tactical' }, systems: { weapons: {
        blips: [{ uuid: 'kestrel', x: 50, z: 50 }],
        banks: ['fore', 'aft'].map(id => ({ id, label: id.toUpperCase(), cooldown: 0 })),
        tubes: [1, 2, 3].map(n => ({ id: `tube${n}`, label: `TUBE ${n}`, loaded_count: 2, target_count: 2, volley_max: 3,
          readiness: { ready: !blocked, blocking_reason: blocked ? ['Loading', 'OutOfArc', 'NoTarget'][n - 1] : 'Ready' },
        })),
        target_uuid: 'kestrel', target_name: 'Kestrel', target_stance: 'hostile', target_class: 'Warship',
        target_bearing: 47, target_range: 312, target_hull_pct: 78, target_shield_freq: 0.41,
        target_shields: (blocked ? ['fore', 'aft', 'port', 'starboard'] : ['fore']).map(label => ({ label, hp: 70, max_hp: 100 })), torpedo_count: 20, torpedo_max: 20,
      } } }, document);
      await document.fonts.ready;
    }, blocked);
    const fit = await page.evaluate(() => {
      const box = e => { const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height, bottom: r.bottom, right: r.right }; };
      const row = document.querySelector('.console-body');
      const card = document.querySelector('#target-lock-card');
      const rails = [...document.querySelectorAll('.phaser-column, .tube-column')];
      const buttons = [...document.querySelectorAll('ph-phasers-controls, ph-torpedo-controls')].flatMap(e => [...e.shadowRoot.querySelectorAll('button')]);
      const reachable = e => {
        let current = e;
        while (current) {
          const r = current.getBoundingClientRect();
          if (['auto', 'scroll'].includes(getComputedStyle(current).overflowY) && current.scrollHeight > current.clientHeight + 1 && r.height > 44 && r.bottom <= innerHeight + 1) return true;
          current = current.parentElement || current.getRootNode().host;
        }
        return e.getBoundingClientRect().bottom <= innerHeight + 1;
      };
      return {
        row: box(row), card: box(card), rails: rails.map(box), scope: box(document.querySelector('ph-tactical-radar')),
        outerOverflow: row.scrollHeight - row.clientHeight,
        buttons: buttons.map(e => ({ ...box(e), reachable: reachable(e) })),
        tubes: document.querySelector('#torpedo-controls').shadowRoot.querySelectorAll('.tube-row').length,
        banks: document.querySelector('#phasers-controls').shadowRoot.querySelectorAll('.bank-row').length,
        facts: card.shadowRoot.textContent,
        targetInPhaserRail: !!card.closest('.weapons-col'),
      };
    });
    expect(fit.banks).toBe(2);
    expect(fit.tubes).toBe(3);
    expect(fit.outerOverflow).toBeLessThanOrEqual(1);
    expect(Math.abs(fit.scope.width - fit.scope.height)).toBeLessThanOrEqual(1);
    expect(fit.scope.width).toBeGreaterThanOrEqual(Math.min(width, height) / 3);
    for (const button of fit.buttons) {
      expect(button.width).toBeGreaterThanOrEqual(44);
      expect(button.height).toBeGreaterThanOrEqual(44);
      expect(button.reachable).toBe(true);
    }
    // Scroll reachability alone hid two landscape tubes below the viewport.
    if (!blocked || width > height) {
      for (const button of fit.buttons) expect(button.bottom).toBeLessThanOrEqual(height);
    }
    for (const fact of ['Kestrel', 'Warship', '78%', '70%', '41%']) expect(fit.facts).toContain(fact);
    expect(fit.targetInPhaserRail).toBe(true);
    if (height > width) {
      expect(fit.card.x).toBe(fit.row.x);
      expect(fit.card.width).toBe(fit.row.width);
      expect(fit.card.bottom).toBeLessThanOrEqual(height);
      expect(fit.rails[0].y).toBe(fit.rails[1].y);
      expect(fit.rails[0].right).toBeLessThan(fit.rails[1].x);
      expect(fit.scope.bottom).toBeLessThan(fit.rails[0].y);
      // The ordinary populated state needs no rail scrolling either.
      if (!blocked) {
        for (const button of fit.buttons) expect(button.bottom).toBeLessThanOrEqual(fit.card.y);
      } else {
        // Reason rows and every shield facing may lengthen the bounded rail.
        // Prove the last control can actually be brought above the target strip.
        const lastFire = page.locator('ph-torpedo-controls button').last();
        await lastFire.scrollIntoViewIfNeeded();
        const button = await lastFire.boundingBox();
        expect(button.y + button.height).toBeLessThanOrEqual(fit.card.y);
        await expect(lastFire).toBeInViewport();
      }
    }
  });
}
}
