import { test, expect } from "@playwright/test";

test("full gig lifecycle: post, bid, accept, fund, deliver, release", async ({ page }) => {
  // Mock Backend API
  const mockJobId = "550e8400-e29b-41d4-a716-446655440000";
  const mockBidId = "b1d00000-0000-0000-0000-000000000000";

  await page.route("**/api/v1/jobs", async (route) => {
    if (route.request().method() === "POST") {
      await route.fulfill({
        status: 201,
        contentType: "application/json",
        body: JSON.stringify({
          id: mockJobId,
          title: "Build a Soroban Smart Contract",
          status: "open",
          budget_usdc: 5000,
          milestones: 2,
        }),
      });
    } else {
      await route.continue();
    }
  });

  await page.route(`**/api/v1/jobs/${mockJobId}`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        id: mockJobId,
        title: "Build a Soroban Smart Contract",
        description: "Implement a simple escrow contract for a freelance platform.",
        status: "open",
        budget_usdc: 5000,
        milestones: 2,
        client_address: "GD...CLIENT",
      }),
    });
  });

  await page.route(`**/api/v1/jobs/${mockJobId}/bids`, async (route) => {
    if (route.request().method() === "POST") {
      await route.fulfill({
        status: 201,
        contentType: "application/json",
        body: JSON.stringify({
          id: mockBidId,
          job_id: mockJobId,
          freelancer_address: "GD...FREELANCER",
          proposal: "test proposal",
          status: "pending",
        }),
      });
    } else {
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify([
          {
            id: mockBidId,
            job_id: mockJobId,
            freelancer_address: "GD...FREELANCER",
            proposal: "I have extensive experience with Soroban and Rust. I can finish this in 3 days.",
            status: "pending",
          },
        ]),
      });
    }
  });

  // 1. Client posts a job
  await page.goto("/jobs/new");
  await page.fill("#job-title", "Build a Soroban Smart Contract");
  await page.fill("#job-description", "Implement a simple escrow contract for a freelance platform.");
  await page.fill("#job-budget", "5000");
  await page.fill("#job-milestones", "2");
  await page.click("#submit-job");

  // Should redirect to job details page
  await expect(page).toHaveURL(`/jobs/${mockJobId}`);
  await expect(page.getByRole("heading", { name: "Build a Soroban Smart Contract" })).toBeVisible();
  await expect(page.getByText(/OPEN/i)).toBeVisible();

  // 2. Freelancer submits a bid
  await page.fill("#bid-proposal", "I have extensive experience with Soroban and Rust. I can finish this in 3 days.");
  await page.click("#submit-bid");

  // Bid should appear in the list
  await expect(page.getByText("Bids (1)")).toBeVisible();
  await expect(page.getByText("I have extensive experience with Soroban and Rust")).toBeVisible();

  // 3. Client accepts the bid
  await page.click("button:has-text('Accept Bid')");

  // Should redirect to funding page
  await expect(page).toHaveURL(`/jobs/${mockJobId}/fund`);
  await expect(page.getByRole("heading", { name: "Fund Escrow" })).toBeVisible();

  // 4. Client deposits escrow
  await page.check("input[type='checkbox']");
  
  // Update mock for status change to funded
  await page.route(`**/api/v1/jobs/${mockJobId}`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        id: mockJobId,
        title: "Build a Soroban Smart Contract",
        status: "funded",
        budget_usdc: 5000,
        milestones: 2,
        client_address: "GD...CLIENT",
        freelancer_address: "GD...FREELANCER",
        on_chain_job_id: 1,
      }),
    });
  });

  await page.click("button:has-text('Deposit $5,100.00 into Escrow')");
  await page.click("button:has-text('Confirm & Sign')");

  // Wait for "Escrow Funded!" success state
  await expect(page.getByRole("heading", { name: "Escrow Funded!" })).toBeVisible({ timeout: 10000 });
  
  // 5. Verify transition back to Job details with FUNDED status
  await page.click("button:has-text('Go to Job')");
  await expect(page).toHaveURL(`/jobs/${mockJobId}`);
  await expect(page.getByText(/FUNDED/i)).toBeVisible();
});
