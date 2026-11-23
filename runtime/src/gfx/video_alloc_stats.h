// SPDX-License-Identifier: GPL-3.0-or-later
// What the video memory allocator (VMA) holds against what it has handed out. Filled by gfx::Device::alloc_stats (vk_device.cpp);
// the text is here so a test can read it without a device.
#pragma once

#include <cstdint>
#include <cstdio>

namespace nxrt::gfx {

struct VideoAllocGroup {
    uint64_t block_bytes = 0;   // memory taken from the driver (VMA's blocks and dedicated allocations)
    uint64_t alloc_bytes = 0;   // of it handed out to images and buffers
    uint32_t blocks = 0;
    uint32_t allocs = 0;
    uint64_t free_bytes() const { return block_bytes > alloc_bytes ? block_bytes - alloc_bytes : 0; }
};

struct VideoAllocStats {
    bool valid = false;
    VideoAllocGroup video;        // the largest device-local heap (small-texture pools included)
    VideoAllocGroup other;        // every other heap (host-visible memory: staging, mapped buffers)
    VideoAllocGroup small;        // of `video` and `other`: the small-texture pools (16 MiB blocks)
    uint32_t video_free_ranges = 0;       // free ranges inside the video heap's blocks
    uint64_t video_largest_free = 0;      // the largest of them
    uint64_t driver_bytes = 0;            // every vkAllocateMemory alive (the gauge), the allocator's and the others'
    // Not from the allocator: host-memory imports of the game's pools, sparse texture chunks, the upscalers' and the overlay's own.
    uint64_t outside_bytes() const {
        const uint64_t in = video.block_bytes + other.block_bytes;
        return driver_bytes > in ? driver_bytes - in : 0;
    }
};

inline int video_alloc_free_percent(const VideoAllocGroup& g) {
    return g.block_bytes ? static_cast<int>((g.free_bytes() * 100 + g.block_bytes / 2) / g.block_bytes) : 0;
}

// One log line. Returns the length written (0: no figures).
inline int video_alloc_text(const VideoAllocStats& s, char* out, size_t n) {
    if (!out || !n) return 0;
    out[0] = 0;
    if (!s.valid) return 0;
    const double mib = 1048576.0;
    return std::snprintf(out, n,
                         "allocator: video heap %u blocks / %.0f MiB taken, %u allocations / %.0f MiB handed out, %.0f MiB free inside the blocks (%d%%, "
                         "%u ranges, largest %.0f MiB); of it small-texture pools %u blocks / %.0f MiB, %.0f MiB free; other heaps %.0f MiB taken, "
                         "%.0f MiB free; outside the allocator %.0f MiB",
                         s.video.blocks, s.video.block_bytes / mib, s.video.allocs, s.video.alloc_bytes / mib, s.video.free_bytes() / mib,
                         video_alloc_free_percent(s.video), s.video_free_ranges, s.video_largest_free / mib, s.small.blocks, s.small.block_bytes / mib,
                         s.small.free_bytes() / mib, s.other.block_bytes / mib, s.other.free_bytes() / mib, s.outside_bytes() / mib);
}

}  // namespace nxrt::gfx
