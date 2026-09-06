"""Exercise the real timeline focus bridge on synthetic DOM; no API writes."""
from playwright.sync_api import sync_playwright, expect

with sync_playwright() as p:
    browser = p.chromium.launch(channel='msedge', headless=True)
    page = browser.new_page(viewport={'width': 1280, 'height': 900})
    page.goto('http://127.0.0.1:3005', wait_until='domcontentloaded')
    page.wait_for_function('!!window.Jx3TimelineBridge')
    page.evaluate('''() => {
      window.Jx3Nav.switchPage('page-sim');
      const panel = document.querySelector('#sim_sequence');
      panel.replaceChildren();
      panel.style.width = '360px';
      for (let i = 0; i < 18; i++) {
        const skill = document.createElement('div');
        skill.className = 'sim-seq-item';
        skill.dataset.sequenceIndex = i;
        skill.style.width = '48px';
        skill.style.height = '48px';
        skill.textContent = i === 8 ? '绝刀' : '盾击';
        panel.appendChild(skill);
      }
      window.Jx3TimelineBridge.focus({kind:'operation',start:8,end:8});
    }''')
    selected = page.locator('#sim_sequence .agent-axis-focus')
    expect(selected).to_have_count(1)
    expect(page.locator('.agent-axis-row-band')).to_have_count(1)
    assert selected.get_attribute('data-sequence-index') == '8'
    assert selected.evaluate('e => getComputedStyle(e).outlineStyle') == 'solid'
    band = page.locator('.agent-axis-row-band')
    assert band.evaluate('e => getComputedStyle(e).pointerEvents') == 'none'
    rect = selected.bounding_box()
    background = band.bounding_box()
    assert background['y'] <= rect['y'] and background['y'] + background['height'] >= rect['y'] + rect['height']
    page.emulate_media(reduced_motion='reduce')
    assert selected.evaluate('e => getComputedStyle(e).animationName') == 'none'
    assert band.evaluate('e => getComputedStyle(e).animationName') == 'none'
    # Theme token changes must affect both the row and target, with no literal color.
    for rgb in ['150, 110, 65', '220, 180, 110']:
        page.evaluate('(rgb) => document.body.style.setProperty("--gold-rgb", rgb)', rgb)
        assert rgb in band.evaluate('e => getComputedStyle(e).backgroundColor')
    page.evaluate('window.Jx3TimelineBridge.clearFocus()')
    expect(selected).to_have_count(0)
    expect(band).to_have_count(0)
    print('OK: real focus bridge, row band, exact target, theme tokens, reduced motion, cleanup')
    browser.close()
