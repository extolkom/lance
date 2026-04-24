import { test, expect } from '@playwright/test';

test.describe('Client Dashboard', () => {
  test.beforeEach(async ({ page }) => {
    // Set role to client
    await page.goto('/');
    await page.evaluate(() => {
      localStorage.setItem('lance-auth-store', JSON.stringify({
        state: {
          role: 'client',
          isLoggedIn: true,
          user: { name: 'Amina O.', email: 'client@lance.so' },
          hydrated: true
        },
        version: 0
      }));
    });
    await page.goto('/');
  });

  test('should display client metrics and active registry', async ({ page }) => {
    await expect(page.locator('h1')).toContainText('Manage hiring and escrow milestones');
    
    // Check stats
    await expect(page.locator('text=Active Jobs')).toBeVisible();
    await expect(page.locator('text=Escrow Volume')).toBeVisible();
    
    // Check active registry
    await expect(page.locator('h2')).toContainText('Active Registry');
    await expect(page.locator('div[class*="group flex items-center justify-between"]')).toHaveCount(5);
  });
});
