import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("kizumail.theme", "dark");
  });
});

test("inbox dropdowns are readable, keyboard accessible and viewport-safe", async ({ page }) => {
  const consoleErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });

  await page.goto("/");
  await expect(page).toHaveTitle("KizuMail");
  await expect(page.getByRole("heading", { name: "Posteingang" })).toBeVisible();

  const filterTrigger = page.getByRole("button", { name: "Mail-Liste filtern" });
  await filterTrigger.click();
  const listbox = page.getByRole("listbox", { name: "Mail-Liste filtern" });
  await expect(listbox).toBeVisible();
  await expect(page.getByRole("option", { name: /Ungelesen/ })).toBeVisible();

  const menuBox = await listbox.locator("..").boundingBox();
  expect(menuBox).not.toBeNull();
  expect(menuBox!.x).toBeGreaterThanOrEqual(8);
  expect(menuBox!.y).toBeGreaterThanOrEqual(8);
  expect(menuBox!.x + menuBox!.width).toBeLessThanOrEqual(1536 - 8);
  expect(menuBox!.y + menuBox!.height).toBeLessThanOrEqual(960 - 8);

  await listbox.press("ArrowDown");
  await listbox.press("Enter");
  await expect(filterTrigger).toContainText("Ungelesen");
  await expect(listbox).toBeHidden();
  expect(consoleErrors).toEqual([]);
});

test("dark and light themes keep controls readable", async ({ page }) => {
  await page.goto("/");
  const themeButton = page.getByRole("button", { name: "Client-Theme umschalten" });
  const brandIcon = page.locator(".brand-icon");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await expect(brandIcon).toHaveAttribute("src", /kizumail-icon-dark/);
  await themeButton.click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(brandIcon).toHaveAttribute("src", /kizumail-icon-light/);

  await page.getByRole("button", { name: "Mail-Liste sortieren" }).click();
  const listbox = page.getByRole("listbox", { name: "Mail-Liste sortieren" });
  await expect(listbox).toBeVisible();
  await expect(page.getByRole("option", { name: /Neueste/ })).toBeVisible();

  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(1);
});

test("light settings use one scroll surface without overlapping nested panels", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Client-Theme umschalten" }).click();
  await page.getByRole("button", { name: "Einstellungen", exact: true }).click();

  const settingsPanel = page.locator(".settings-panel");
  await expect(settingsPanel).toBeVisible();
  const layout = await settingsPanel.evaluate((element) => {
    const panel = element.getBoundingClientRect();
    const lastChild = element.lastElementChild?.getBoundingClientRect();
    const scrollSurface = element.parentElement;
    return {
      panelBottom: panel.bottom,
      lastChildBottom: lastChild?.bottom ?? panel.bottom,
      panelScrollHeight: element.scrollHeight,
      panelClientHeight: element.clientHeight,
      readerScrollHeight: scrollSurface?.scrollHeight ?? 0,
      readerClientHeight: scrollSurface?.clientHeight ?? 0,
      readerOverflowY: scrollSurface ? getComputedStyle(scrollSurface).overflowY : ""
    };
  });

  expect(layout.lastChildBottom).toBeLessThanOrEqual(layout.panelBottom + 1);
  expect(layout.panelScrollHeight).toBeLessThanOrEqual(layout.panelClientHeight + 1);
  expect(layout.readerScrollHeight).toBeGreaterThan(layout.readerClientHeight);
  expect(layout.readerOverflowY).toBe("auto");
});

test("light mode stays readable across inbox and workspace sections", async ({ page }) => {
  const consoleErrors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") consoleErrors.push(message.text());
  });

  await page.goto("/");
  await page.getByRole("button", { name: "Client-Theme umschalten" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");

  const assertReadableSurface = async (selector: string) => {
    const colors = await page.locator(selector).evaluate((element) => {
      const style = getComputedStyle(element);
      const heading = element.querySelector("h1, h2, h3, strong");
      return {
        surface: style.backgroundColor,
        text: style.color,
        heading: heading ? getComputedStyle(heading).color : style.color,
        overflow: document.documentElement.scrollWidth - document.documentElement.clientWidth
      };
    });
    expect(colors.text).not.toBe("rgb(247, 248, 252)");
    expect(colors.heading).not.toBe("rgb(255, 255, 255)");
    expect(colors.overflow).toBeLessThanOrEqual(1);
  };

  await assertReadableSurface(".inbox-panel");

  await page.getByRole("button", { name: "Newsletter Neu" }).click();
  await expect(page.locator(".newsletter-panel")).toBeVisible();
  await assertReadableSurface(".newsletter-panel");

  await page.getByRole("button", { name: "Regeln", exact: true }).click();
  await expect(page.locator(".rules-panel")).toBeVisible();
  await assertReadableSurface(".rules-panel");

  await page.getByRole("button", { name: "Einstellungen", exact: true }).click();
  await expect(page.locator(".settings-panel")).toBeVisible();
  await assertReadableSurface(".settings-panel");
  expect(consoleErrors).toEqual([]);
});

test("custom domains can explicitly choose Microsoft 365 OAuth", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Konto verbinden" }).first().click();
  await page.getByLabel("E-Mail-Adresse").fill("name@firma.example");
  await expect(page.getByRole("button", { name: /Microsoft 365 \/ Outlook/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /Google Workspace/ })).toBeVisible();
});

test("mail renderer discovers remote media and blocks it until explicitly allowed", async ({ page }) => {
  await page.goto("/");
  const result = await page.evaluate(async () => {
    const rendering = await import("/src/lib/mailRendering.ts");
    const message = {
      id: "media-test",
      accountId: "account-test",
      sender: "Media Test",
      senderEmail: "mail@example.test",
      subject: "Media",
      preview: "Media preview",
      bodyText: "Media preview",
      bodyHtml: `
        <html><body>
          <img src="https://cdn.example.test/hero.jpg" alt="Hero">
          <img src="https://tracking.example.test/open.gif" width="1" height="1">
          <img src="cid:logo@example.test" alt="Logo">
          <video src="https://cdn.example.test/movie.mp4"></video>
          <a href="https://cdn.example.test/document.pdf">Dokument</a>
        </body></html>`,
      date: "2026-07-15T12:00:00Z",
      folder: "INBOX",
      unread: false,
      flagged: false,
      hasAttachments: true,
      attachments: [{
        filename: "logo.png",
        mimeType: "image/png",
        size: 5,
        contentId: "logo@example.test",
        storageRef: "C:\\KizuMail\\attachments\\logo.png",
        inline: true
      }],
      hasNewsletterSignals: false
    };
    const media = rendering.extractRemoteEmailMedia(message);
    const blocked = new DOMParser().parseFromString(
      rendering.buildOriginalEmailDocument(message, false),
      "text/html"
    );
    const allowed = new DOMParser().parseFromString(
      rendering.buildOriginalEmailDocument(message, true),
      "text/html"
    );
    return {
      kinds: media.map((item) => item.kind),
      blockedRemoteSrc: blocked.querySelector('img[alt="Hero"]')?.getAttribute("src"),
      blockedMarker: blocked.querySelector('img[alt="Hero"]')?.getAttribute("data-kizumail-blocked-src"),
      allowedRemoteSrc: allowed.querySelector('img[alt="Hero"]')?.getAttribute("src"),
      localCidSrc: blocked.querySelector('img[alt="Logo"]')?.getAttribute("src"),
      trackingIncluded: media.some((item) => item.url.includes("tracking.example.test"))
    };
  });

  expect(result.kinds).toEqual(["image", "video", "pdf"]);
  expect(result.blockedRemoteSrc).toBeNull();
  expect(result.blockedMarker).toBe("https://cdn.example.test/hero.jpg");
  expect(result.allowedRemoteSrc).toBe("https://cdn.example.test/hero.jpg");
  expect(result.localCidSrc).not.toContain("cid:");
  expect(result.trackingIncluded).toBe(false);
});
