// SPDX-License-Identifier: GPL-3.0-or-later
// Where the FSR 3 frame generation swapchain's three queues of its own come from (framegen_ffx.cpp, made by vk_device.cpp): present,
// image acquire and async compute, each a queue no one else submits to. Free of Vulkan, so the CPU tests (tests/test_framegen_queues.cpp)
// decide a device's queues the same way.
//
// With 4 or more queues in the game's family (graphics, compute and present) all three come from it, as before. Mesa's RADV (Linux, Proton)
// has a single graphics queue, so the three then come from other families as FFX's VkQueueInfoFFXAPI allows (its family index is what
// FFX's queue family ownership transfers use): async compute from a compute family other than the game's, present from any family that
// presents to the window and can copy, image acquire from any family that can submit. Families that are video only or protected are left
// out (a protected queue needs a flag the device does not ask for).
#pragma once
#include <cstdint>
#include <string>
#include <vector>

namespace nxrt::extras::fgq {

// VkQueueFlagBits values (vulkan_core.h)
constexpr uint32_t kGraphics = 0x1, kCompute = 0x2, kTransfer = 0x4, kProtected = 0x10;

struct Family {
    uint32_t flags = 0;      // VkQueueFamilyProperties::queueFlags
    uint32_t count = 0;      // queueCount
    bool present = false;    // vkGetPhysicalDeviceSurfaceSupportKHR for the window
};

struct Queue {
    uint32_t family = 0, index = 0;
};

struct Plan {
    bool on = false;
    bool cross_family = false;   // one of the three is outside the game's family
    Queue present, acquire, compute;
    std::string why;             // !on: what was missing (for the log)
};

inline Plan plan(const std::vector<Family>& fams, uint32_t game) {
    Plan p;
    if (game >= fams.size()) {
        p.why = "the game's queue family is unknown";
        return p;
    }
    if (fams[game].count >= 4) {
        p.on = true;
        p.present = {game, 1};
        p.acquire = {game, 2};
        p.compute = {game, 3};
        return p;
    }
    // the next free queue of each family (the game's first queue is the renderer's)
    std::vector<uint32_t> next(fams.size(), 0);
    next[game] = 1;
    auto usable = [&](uint32_t f) { return !(fams[f].flags & kProtected) && (fams[f].flags & (kGraphics | kCompute | kTransfer)) && next[f] < fams[f].count; };
    auto take = [&](uint32_t f) { return Queue{f, next[f]++}; };
    auto find = [&](auto&& ok, bool game_first) -> int {
        if (game_first && usable(game) && ok(game)) return static_cast<int>(game);
        for (uint32_t f = 0; f < fams.size(); ++f)
            if (f != game && usable(f) && ok(f)) return static_cast<int>(f);
        if (!game_first && usable(game) && ok(game)) return static_cast<int>(game);
        return -1;
    };
    // present first: it is the most particular (a queue that can copy and present to the window)
    const int pf = find([&](uint32_t f) { return fams[f].present; }, true);
    if (pf < 0) {
        p.why = "no free queue that presents to the window";
        return p;
    }
    p.present = take(static_cast<uint32_t>(pf));
    // async compute: a compute family other than the game's is what it is for
    const int cf = find([&](uint32_t f) { return (fams[f].flags & kCompute) != 0; }, false);
    if (cf < 0) {
        p.why = "no free compute queue";
        return p;
    }
    p.compute = take(static_cast<uint32_t>(cf));
    const int af = find([](uint32_t) { return true; }, true);
    if (af < 0) {
        p.why = "no free queue for the image acquire";
        return p;
    }
    p.acquire = take(static_cast<uint32_t>(af));
    p.on = true;
    p.cross_family = p.present.family != game || p.acquire.family != game || p.compute.family != game;
    return p;
}

// The number of queues to create in family f (the game's included: 1 + its extras).
inline uint32_t queues_in(const Plan& p, uint32_t f, uint32_t game) {
    uint32_t n = f == game ? 1u : 0u;
    if (!p.on) return n;
    for (const Queue& q : {p.present, p.acquire, p.compute})
        if (q.family == f && q.index + 1 > n) n = q.index + 1;
    return n;
}

}  // namespace nxrt::extras::fgq
