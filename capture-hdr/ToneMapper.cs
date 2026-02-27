// ============================================================================
// ToneMapper.cs — HDR (half-float) to SDR (8-bit sRGB) conversion
// Uses Extended Reinhard (luminance-preserving) tone mapping.
// SDR passthrough: when all pixel values are in [0,1], Reinhard barely
// changes them (L/(1+L) ≈ L for small L), so this works on SDR too.
// ============================================================================

using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;

namespace CaptureHdr;

/// <summary>
/// Provides HDR-to-SDR tone mapping using the Extended Reinhard operator.
/// Converts R16G16B16A16_Float (half-float) pixel data to B8G8R8A8 (8-bit sRGB).
/// </summary>
public static class ToneMapper
{
    // ======================== CONSTANTS ========================

    /// <summary>Rec.709 luminance coefficient for red channel.</summary>
    private const float LUM_R = 0.2126f;

    /// <summary>Rec.709 luminance coefficient for green channel.</summary>
    private const float LUM_G = 0.7152f;

    /// <summary>Rec.709 luminance coefficient for blue channel.</summary>
    private const float LUM_B = 0.0722f;

    /// <summary>sRGB linear-to-gamma threshold.</summary>
    private const float SRGB_THRESHOLD = 0.0031308f;

    /// <summary>Bytes per pixel in R16G16B16A16_Float format (4 channels x 2 bytes).</summary>
    private const int BYTES_PER_HALF_PIXEL = 8;

    /// <summary>Bytes per pixel in B8G8R8A8 format (4 channels x 1 byte).</summary>
    private const int BYTES_PER_BGRA_PIXEL = 4;

    // ======================== PUBLIC API ========================

    /// <summary>
    /// Tone maps HDR half-float pixel data to 8-bit sRGB BGRA.
    /// Uses Extended Reinhard (luminance-preserving) with sRGB gamma encoding.
    /// Processes scanlines in parallel for performance.
    /// </summary>
    /// <param name="halfFloatPixels">
    /// Raw pixel data in R16G16B16A16_Float format (8 bytes per pixel).
    /// Layout: [R_half, G_half, B_half, A_half] repeated for each pixel.
    /// </param>
    /// <param name="width">Image width in pixels.</param>
    /// <param name="height">Image height in pixels.</param>
    /// <returns>Byte array in B8G8R8A8 format (4 bytes per pixel), suitable for Windows bitmaps.</returns>
    /// <exception cref="ArgumentException">Thrown when pixel data size does not match width * height.</exception>
    public static byte[] ToneMap(byte[] halfFloatPixels, int width, int height)
    {
        int expectedSize = width * height * BYTES_PER_HALF_PIXEL;
        if (halfFloatPixels.Length < expectedSize)
        {
            throw new ArgumentException(
                $"Pixel data too small: got {halfFloatPixels.Length} bytes, " +
                $"expected {expectedSize} for {width}x{height} R16G16B16A16_Float");
        }

        byte[] output = new byte[width * height * BYTES_PER_BGRA_PIXEL];

        // Process scanlines in parallel for multi-threaded performance.
        // For 4K (3840x2160), this divides work across all available cores.
        Parallel.For(0, height, y =>
        {
            ToneMapScanline(halfFloatPixels, output, y, width);
        });

        return output;
    }

    /// <summary>
    /// Tone maps HDR half-float pixel data to 8-bit sRGB BGRA, reading from a span.
    /// Overload for callers that have a <see cref="ReadOnlySpan{T}"/> instead of an array.
    /// </summary>
    /// <param name="halfFloatPixels">Source pixel span in R16G16B16A16_Float format.</param>
    /// <param name="width">Image width in pixels.</param>
    /// <param name="height">Image height in pixels.</param>
    /// <returns>Byte array in B8G8R8A8 format (4 bytes per pixel).</returns>
    public static byte[] ToneMap(ReadOnlySpan<byte> halfFloatPixels, int width, int height)
    {
        // Copy to array for Parallel.For (spans cannot cross thread boundaries)
        byte[] pixelArray = halfFloatPixels.ToArray();
        return ToneMap(pixelArray, width, height);
    }

    // ======================== SCANLINE PROCESSING ========================

    /// <summary>
    /// Processes a single scanline, converting each pixel from HDR half-float to SDR BGRA.
    /// Uses unsafe pointer access for performance-critical inner loop.
    /// </summary>
    /// <param name="src">Source pixel data (R16G16B16A16_Float).</param>
    /// <param name="dst">Destination pixel data (B8G8R8A8).</param>
    /// <param name="y">Scanline index (row number).</param>
    /// <param name="width">Width of the image in pixels.</param>
    private static unsafe void ToneMapScanline(byte[] src, byte[] dst, int y, int width)
    {
        int srcOffset = y * width * BYTES_PER_HALF_PIXEL;
        int dstOffset = y * width * BYTES_PER_BGRA_PIXEL;

        fixed (byte* pSrc = &src[srcOffset])
        fixed (byte* pDst = &dst[dstOffset])
        {
            ushort* halfPtr = (ushort*)pSrc;
            byte* outPtr = pDst;

            for (int x = 0; x < width; x++)
            {
                // Step 1: Decode half-float to float (R, G, B, A)
                float r = HalfToFloat(halfPtr[0]);
                float g = HalfToFloat(halfPtr[1]);
                float b = HalfToFloat(halfPtr[2]);
                float a = HalfToFloat(halfPtr[3]);

                // Step 2: Compute Rec.709 luminance
                float lum = LUM_R * r + LUM_G * g + LUM_B * b;

                // Step 3: Handle zero/negative luminance — output black
                if (lum <= 0.0f)
                {
                    outPtr[0] = 0;   // B
                    outPtr[1] = 0;   // G
                    outPtr[2] = 0;   // R
                    outPtr[3] = 255; // A (fully opaque)
                }
                else
                {
                    // Step 4: Extended Reinhard tone mapping on luminance
                    // L_mapped = L / (1 + L)
                    // For SDR content (L < 1), this is nearly identity: 0.5/(1+0.5) = 0.333
                    // For HDR content (L > 1), this compresses: 2.0/(1+2.0) = 0.667
                    float lumMapped = lum / (1.0f + lum);

                    // Step 5: Scale RGB channels by luminance ratio
                    float scale = lumMapped / lum;
                    float rMapped = r * scale;
                    float gMapped = g * scale;
                    float bMapped = b * scale;

                    // Step 6: Clamp to [0, 1]
                    rMapped = Clamp01(rMapped);
                    gMapped = Clamp01(gMapped);
                    bMapped = Clamp01(bMapped);

                    // Step 7: Apply sRGB gamma curve (linear → sRGB)
                    rMapped = LinearToSrgb(rMapped);
                    gMapped = LinearToSrgb(gMapped);
                    bMapped = LinearToSrgb(bMapped);

                    // Step 8: Quantize to 8-bit with rounding
                    // Step 9: Store in BGRA order (Windows bitmap convention)
                    outPtr[0] = FloatToByte(bMapped); // B
                    outPtr[1] = FloatToByte(gMapped); // G
                    outPtr[2] = FloatToByte(rMapped); // R
                    outPtr[3] = FloatToByte(Clamp01(a)); // A
                }

                // Advance pointers: 4 half-floats (8 bytes) in, 4 bytes out
                halfPtr += 4;
                outPtr += BYTES_PER_BGRA_PIXEL;
            }
        }
    }

    // ======================== HELPER FUNCTIONS ========================

    /// <summary>
    /// Converts a 16-bit IEEE 754 half-precision float to a 32-bit float.
    /// Uses <see cref="System.Half"/> available in .NET 6+.
    /// </summary>
    /// <param name="halfBits">Raw 16-bit half-float value.</param>
    /// <returns>The equivalent 32-bit float value.</returns>
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static float HalfToFloat(ushort halfBits)
    {
        // System.Half handles all edge cases: denormals, inf, NaN
        return (float)BitConverter.ToHalf(BitConverter.GetBytes(halfBits), 0);
    }

    /// <summary>
    /// Applies the sRGB transfer function (linear to gamma-encoded).
    /// Uses the official IEC 61966-2-1 piecewise formula.
    /// </summary>
    /// <param name="linear">Linear-light value in [0, 1].</param>
    /// <returns>sRGB gamma-encoded value in [0, 1].</returns>
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static float LinearToSrgb(float linear)
    {
        // Piecewise sRGB gamma curve:
        // - Below threshold: simple linear scaling (avoids pow() for dark values)
        // - Above threshold: standard gamma power curve
        if (linear <= SRGB_THRESHOLD)
        {
            return 12.92f * linear;
        }

        return 1.055f * MathF.Pow(linear, 1.0f / 2.4f) - 0.055f;
    }

    /// <summary>
    /// Clamps a float to the [0, 1] range.
    /// Handles NaN by treating it as 0 (NaN comparisons are false).
    /// </summary>
    /// <param name="value">Input value.</param>
    /// <returns>Value clamped to [0, 1].</returns>
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static float Clamp01(float value)
    {
        // NaN-safe: if value is NaN, both comparisons are false, so we return 0
        if (value < 0.0f || float.IsNaN(value)) return 0.0f;
        if (value > 1.0f) return 1.0f;
        return value;
    }

    /// <summary>
    /// Quantizes a [0, 1] float to an 8-bit byte with rounding.
    /// </summary>
    /// <param name="value">Float value in [0, 1] (must already be clamped).</param>
    /// <returns>Byte value in [0, 255].</returns>
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static byte FloatToByte(float value)
    {
        // Add 0.5 for rounding (not truncation) to minimize quantization error
        return (byte)(value * 255.0f + 0.5f);
    }
}
