// `test` for every spec: after each test body (pass or fail) it saves a
// full-page screenshot to `test-results/screenshots/<file> - <test>.png`, so
// all of them can be scrolled through in one folder.

import { test as base } from "@playwright/test";
import path from "node:path";

export { expect } from "@playwright/test";

export const test = base.extend<{ saveScreenshot: void }>({
  saveScreenshot: [
    async ({ page }, use, testInfo) => {
      await use();
      const file = path.basename(testInfo.file, ".spec.ts");
      // "/" and other characters some file systems reject become "_".
      const name = `${file} - ${testInfo.title}`.replace(/[/\\:*?"<>|]/g, "_");
      const shot = path.join(testInfo.project.outputDir, "screenshots", `${name}.png`);
      await page.screenshot({ path: shot, fullPage: true });
    },
    { auto: true },
  ],
});
