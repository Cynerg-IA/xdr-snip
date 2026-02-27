// ============================================================================
// Program.cs — CLI entry point and HDR screen capture pipeline
//
// Uses Windows.Graphics.Capture (WGC) to grab frames in R16G16B16A16Float,
// tone maps via ToneMapper, and encodes to JPEG.
//
// Exit codes: 0=success, 1=capture failed, 2=encoding failed, 3=invalid args
// ============================================================================

using System.Runtime.InteropServices;
using System.Runtime.InteropServices.WindowsRuntime;
using Windows.Graphics;
using Windows.Graphics.Capture;
using Windows.Graphics.DirectX;
using Windows.Graphics.DirectX.Direct3D11;
using Windows.Graphics.Imaging;
using Windows.Storage.Streams;

namespace CaptureHdr;

/// <summary>
/// Main entry point for the HDR screen capture tool.
/// Orchestrates the full pipeline: CLI parsing, monitor enumeration, frame capture,
/// tone mapping, and JPEG encoding.
/// </summary>
internal static class Program
{
    // ======================== EXIT CODES ========================

    private const int EXIT_SUCCESS = 0;
    private const int EXIT_CAPTURE_FAILED = 1;
    private const int EXIT_ENCODING_FAILED = 2;
    private const int EXIT_INVALID_ARGS = 3;

    // ======================== DEFAULT VALUES ========================

    /// <summary>Default JPEG quality if not specified on CLI.</summary>
    private const int DEFAULT_QUALITY = 90;

    /// <summary>Maximum wait time for a capture frame (milliseconds).</summary>
    private const int FRAME_TIMEOUT_MS = 5000;

    // ======================== MAIN ========================

    /// <summary>
    /// Application entry point. Parses CLI args and runs the capture pipeline.
    /// </summary>
    /// <returns>Exit code: 0=success, 1=capture failed, 2=encoding failed, 3=invalid args.</returns>
    static int Main(string[] args)
    {
        // Step 1: Parse CLI arguments
        if (!TryParseArgs(args, out var options))
        {
            return EXIT_INVALID_ARGS;
        }

        try
        {
            // Run the async capture pipeline synchronously.
            // We create a DispatcherQueue on this thread so WGC callbacks work.
            return RunCaptureAsync(options).GetAwaiter().GetResult();
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"[capture-hdr] Fatal error: {ex.Message}");
            Console.Error.WriteLine($"[capture-hdr] Stack trace: {ex.StackTrace}");
            return EXIT_CAPTURE_FAILED;
        }
    }

    // ======================== CAPTURE PIPELINE ========================

    /// <summary>
    /// Runs the full capture pipeline asynchronously:
    /// 1. Create DispatcherQueue (required by WGC)
    /// 2. Enumerate monitors to find the target HMONITOR
    /// 3. Create GraphicsCaptureItem from HMONITOR
    /// 4. Create Direct3D device and frame pool
    /// 5. Capture one frame
    /// 6. Crop to requested region
    /// 7. Tone map HDR to SDR
    /// 8. Encode as JPEG
    /// </summary>
    /// <param name="options">Parsed CLI options.</param>
    /// <returns>Exit code.</returns>
    private static async Task<int> RunCaptureAsync(CaptureOptions options)
    {
        // Step 2: Create a DispatcherQueue on this thread.
        // WGC frame pool events are dispatched via this queue.
        Console.Error.WriteLine("[capture-hdr] Creating DispatcherQueue...");
        var controller = CreateDispatcherQueueController();
        if (controller == IntPtr.Zero)
        {
            Console.Error.WriteLine("[capture-hdr] ERROR: Failed to create DispatcherQueueController");
            return EXIT_CAPTURE_FAILED;
        }

        // Step 3: Enumerate monitors to find the target
        Console.Error.WriteLine($"[capture-hdr] Enumerating monitors (target index: {options.MonitorIndex})...");
        var monitors = EnumerateMonitors();
        Console.Error.WriteLine($"[capture-hdr] Found {monitors.Count} monitor(s)");

        if (options.MonitorIndex < 0 || options.MonitorIndex >= monitors.Count)
        {
            Console.Error.WriteLine(
                $"[capture-hdr] ERROR: Monitor index {options.MonitorIndex} out of range. " +
                $"Available: 0-{monitors.Count - 1}");
            return EXIT_INVALID_ARGS;
        }

        var targetMonitor = monitors[options.MonitorIndex];
        Console.Error.WriteLine(
            $"[capture-hdr] Target monitor: index={options.MonitorIndex}, " +
            $"hmon=0x{targetMonitor.Hmonitor:X}, " +
            $"bounds=({targetMonitor.Left},{targetMonitor.Top},{targetMonitor.Width}x{targetMonitor.Height})");

        // Step 4: Validate region against monitor bounds
        var region = options.Region ?? new CaptureRegion(0, 0, targetMonitor.Width, targetMonitor.Height);
        if (region.X < 0 || region.Y < 0 ||
            region.X + region.Width > targetMonitor.Width ||
            region.Y + region.Height > targetMonitor.Height)
        {
            Console.Error.WriteLine(
                $"[capture-hdr] ERROR: Region ({region.X},{region.Y},{region.Width}x{region.Height}) " +
                $"exceeds monitor bounds ({targetMonitor.Width}x{targetMonitor.Height})");
            return EXIT_INVALID_ARGS;
        }

        Console.Error.WriteLine(
            $"[capture-hdr] Capture region: ({region.X},{region.Y}) {region.Width}x{region.Height}");

        // Step 5: Create Direct3D device
        Console.Error.WriteLine("[capture-hdr] Creating Direct3D11 device...");
        IDirect3DDevice? d3dDevice = null;
        try
        {
            d3dDevice = CreateDirect3DDevice();
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"[capture-hdr] ERROR: Failed to create D3D11 device: {ex.Message}");
            return EXIT_CAPTURE_FAILED;
        }

        // Step 6: Create GraphicsCaptureItem from HMONITOR
        Console.Error.WriteLine("[capture-hdr] Creating GraphicsCaptureItem from monitor...");
        GraphicsCaptureItem? captureItem;
        try
        {
            captureItem = CreateCaptureItemForMonitor(targetMonitor.Hmonitor);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine(
                $"[capture-hdr] ERROR: Failed to create capture item: {ex.Message}");
            return EXIT_CAPTURE_FAILED;
        }

        if (captureItem == null)
        {
            Console.Error.WriteLine("[capture-hdr] ERROR: CreateCaptureItemForMonitor returned null");
            return EXIT_CAPTURE_FAILED;
        }

        Console.Error.WriteLine(
            $"[capture-hdr] Capture item size: {captureItem.Size.Width}x{captureItem.Size.Height}");

        // Step 7: Create frame pool with HDR pixel format
        // R16G16B16A16Float preserves HDR values above 1.0
        Console.Error.WriteLine("[capture-hdr] Creating frame pool (R16G16B16A16Float)...");
        var framePool = Direct3D11CaptureFramePool.CreateFreeThreaded(
            d3dDevice,
            DirectXPixelFormat.R16G16B16A16Float,
            1, // buffer count — we only need one frame
            captureItem.Size);

        // Step 8: Set up capture session
        var session = framePool.CreateCaptureSession(captureItem);

        // Disable cursor and border capture for clean screenshots
        try { session.IsCursorCaptureEnabled = false; }
        catch { /* Property not available on older Windows builds */ }

        try { session.IsBorderRequired = false; }
        catch { /* Property not available on older Windows builds */ }

        // Step 9: Capture one frame using FrameArrived event
        Console.Error.WriteLine("[capture-hdr] Starting capture session, waiting for frame...");
        Direct3D11CaptureFrame? capturedFrame = null;
        var frameReady = new ManualResetEventSlim(false);

        framePool.FrameArrived += (pool, _) =>
        {
            // Take the frame from the pool
            capturedFrame = pool.TryGetNextFrame();
            if (capturedFrame != null)
            {
                frameReady.Set();
            }
        };

        session.StartCapture();

        // Wait for a frame with timeout
        if (!frameReady.Wait(TimeSpan.FromMilliseconds(FRAME_TIMEOUT_MS)))
        {
            Console.Error.WriteLine(
                $"[capture-hdr] ERROR: Timed out waiting for frame ({FRAME_TIMEOUT_MS}ms)");
            session.Dispose();
            framePool.Dispose();
            return EXIT_CAPTURE_FAILED;
        }

        // Stop session immediately — we only need one frame
        session.Dispose();
        Console.Error.WriteLine("[capture-hdr] Frame captured successfully");

        // Step 10: Get the frame surface and create a SoftwareBitmap
        Console.Error.WriteLine("[capture-hdr] Converting frame surface to SoftwareBitmap...");
        SoftwareBitmap? fullBitmap;
        try
        {
            fullBitmap = await SoftwareBitmap.CreateCopyFromSurfaceAsync(
                capturedFrame!.Surface,
                BitmapAlphaMode.Premultiplied);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine(
                $"[capture-hdr] ERROR: Failed to create SoftwareBitmap from surface: {ex.Message}");
            capturedFrame?.Dispose();
            framePool.Dispose();
            return EXIT_CAPTURE_FAILED;
        }

        // Done with the capture frame and pool
        capturedFrame?.Dispose();
        framePool.Dispose();

        Console.Error.WriteLine(
            $"[capture-hdr] Full bitmap: {fullBitmap.PixelWidth}x{fullBitmap.PixelHeight}, " +
            $"format={fullBitmap.BitmapPixelFormat}");

        // Step 11: Read pixel data and crop to region
        Console.Error.WriteLine("[capture-hdr] Reading pixel data and cropping...");
        byte[] croppedPixels;
        int outputWidth = region.Width;
        int outputHeight = region.Height;
        bool isHdrFormat;

        try
        {
            (croppedPixels, isHdrFormat) = CropPixelData(fullBitmap, region);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"[capture-hdr] ERROR: Failed to crop pixels: {ex.Message}");
            fullBitmap.Dispose();
            return EXIT_CAPTURE_FAILED;
        }

        fullBitmap.Dispose();
        Console.Error.WriteLine(
            $"[capture-hdr] Cropped to {outputWidth}x{outputHeight}, " +
            $"HDR format={isHdrFormat}, data size={croppedPixels.Length} bytes");

        // Step 12: Tone map HDR to SDR
        byte[] sdrPixels;
        if (isHdrFormat)
        {
            Console.Error.WriteLine("[capture-hdr] Tone mapping HDR to SDR (Extended Reinhard)...");
            var sw = System.Diagnostics.Stopwatch.StartNew();
            sdrPixels = ToneMapper.ToneMap(croppedPixels, outputWidth, outputHeight);
            sw.Stop();
            Console.Error.WriteLine($"[capture-hdr] Tone mapping complete in {sw.ElapsedMilliseconds}ms");
        }
        else
        {
            // SDR format — pixels are already 8-bit BGRA, no tone mapping needed
            Console.Error.WriteLine("[capture-hdr] SDR format detected, skipping tone mapping");
            sdrPixels = croppedPixels;
        }

        // Step 13: Encode as JPEG
        Console.Error.WriteLine(
            $"[capture-hdr] Encoding JPEG (quality={options.Quality}) to: {options.OutputPath}");
        try
        {
            await EncodeJpegAsync(sdrPixels, outputWidth, outputHeight, options.Quality, options.OutputPath);
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"[capture-hdr] ERROR: JPEG encoding failed: {ex.Message}");
            return EXIT_ENCODING_FAILED;
        }

        // Verify output file was written
        var fileInfo = new FileInfo(options.OutputPath);
        if (!fileInfo.Exists || fileInfo.Length == 0)
        {
            Console.Error.WriteLine("[capture-hdr] ERROR: Output file is empty or missing");
            return EXIT_ENCODING_FAILED;
        }

        Console.Error.WriteLine(
            $"[capture-hdr] Success: {options.OutputPath} ({fileInfo.Length} bytes)");
        return EXIT_SUCCESS;
    }

    // ======================== MONITOR ENUMERATION ========================

    /// <summary>
    /// Enumerates all display monitors using Win32 EnumDisplayMonitors.
    /// Returns a list of monitor handles with their bounds.
    /// </summary>
    /// <returns>List of <see cref="MonitorInfo"/> for each attached monitor.</returns>
    private static List<MonitorInfo> EnumerateMonitors()
    {
        var monitors = new List<MonitorInfo>();

        // Callback invoked once per monitor by EnumDisplayMonitors
        EnumMonitorsDelegate callback = (IntPtr hMonitor, IntPtr hdcMonitor, ref RECT lprcMonitor, IntPtr dwData) =>
        {
            var info = new MONITORINFOEX();
            info.cbSize = Marshal.SizeOf<MONITORINFOEX>();

            if (GetMonitorInfo(hMonitor, ref info))
            {
                monitors.Add(new MonitorInfo
                {
                    Hmonitor = hMonitor,
                    Left = info.rcMonitor.left,
                    Top = info.rcMonitor.top,
                    Width = info.rcMonitor.right - info.rcMonitor.left,
                    Height = info.rcMonitor.bottom - info.rcMonitor.top,
                    IsPrimary = (info.dwFlags & MONITORINFOF_PRIMARY) != 0
                });
            }

            return true; // Continue enumeration
        };

        // Enumerate all monitors (null HDC = all monitors, null clip rect = no filter)
        EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero, callback, IntPtr.Zero);

        // Sort: primary monitor first, then by position (left-to-right, top-to-bottom)
        monitors.Sort((a, b) =>
        {
            if (a.IsPrimary != b.IsPrimary) return a.IsPrimary ? -1 : 1;
            if (a.Left != b.Left) return a.Left.CompareTo(b.Left);
            return a.Top.CompareTo(b.Top);
        });

        return monitors;
    }

    // ======================== DIRECT3D DEVICE CREATION ========================

    /// <summary>
    /// Creates an <see cref="IDirect3DDevice"/> suitable for Windows.Graphics.Capture.
    /// Uses D3D11CreateDevice with hardware driver, then wraps via DXGI interop.
    /// </summary>
    /// <returns>A WinRT IDirect3DDevice backed by a D3D11 hardware device.</returns>
    /// <exception cref="COMException">Thrown if D3D11 device creation fails.</exception>
    private static IDirect3DDevice CreateDirect3DDevice()
    {
        // Create a D3D11 device with hardware acceleration
        int hr = D3D11CreateDevice(
            IntPtr.Zero,                        // pAdapter: null = default adapter
            D3D_DRIVER_TYPE_HARDWARE,           // hardware GPU
            IntPtr.Zero,                        // software rasterizer (unused)
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,   // required for WGC interop
            null,                               // feature levels: null = default
            0,                                  // num feature levels
            D3D11_SDK_VERSION,                  // SDK version
            out IntPtr d3d11Device,             // output: ID3D11Device*
            out _,                              // output: actual feature level
            out _);                             // output: ID3D11DeviceContext*

        if (hr < 0)
        {
            Marshal.ThrowExceptionForHR(hr);
        }

        // Get the IDXGIDevice from the D3D11 device
        var dxgiDeviceGuid = new Guid("54ec77fa-1377-44e6-8c32-88fd5f44c84c"); // IDXGIDevice
        hr = Marshal.QueryInterface(d3d11Device, ref dxgiDeviceGuid, out IntPtr dxgiDevice);
        Marshal.Release(d3d11Device); // Release the D3D11 device ref (QI added one on dxgiDevice)

        if (hr < 0)
        {
            Marshal.ThrowExceptionForHR(hr);
        }

        // Wrap the DXGI device as a WinRT IDirect3DDevice
        hr = CreateDirect3D11DeviceFromDXGIDevice(dxgiDevice, out IntPtr inspectable);
        Marshal.Release(dxgiDevice);

        if (hr < 0)
        {
            Marshal.ThrowExceptionForHR(hr);
        }

        // Marshal the IInspectable to the WinRT IDirect3DDevice type
        var device = (IDirect3DDevice)Marshal.GetObjectForIUnknown(inspectable);
        Marshal.Release(inspectable);

        return device;
    }

    // ======================== CAPTURE ITEM CREATION ========================

    /// <summary>
    /// Creates a <see cref="GraphicsCaptureItem"/> for the specified monitor handle
    /// using the IGraphicsCaptureItemInterop COM interface.
    /// </summary>
    /// <param name="hMonitor">Win32 HMONITOR handle.</param>
    /// <returns>A GraphicsCaptureItem for the monitor.</returns>
    private static GraphicsCaptureItem? CreateCaptureItemForMonitor(IntPtr hMonitor)
    {
        // Get the activation factory for GraphicsCaptureItem, then QI for interop
        var factoryPtr = GetActivationFactory(
            "Windows.Graphics.Capture.GraphicsCaptureItem",
            typeof(IGraphicsCaptureItemInterop).GUID);

        var interop = (IGraphicsCaptureItemInterop)Marshal.GetObjectForIUnknown(factoryPtr);
        Marshal.Release(factoryPtr);

        // Create a capture item for the monitor
        var captureItemGuid = new Guid("79C3F95B-31F7-4EC2-A464-632EF5D30760"); // IGraphicsCaptureItem
        IntPtr rawItem = interop.CreateForMonitor(hMonitor, ref captureItemGuid);

        if (rawItem == IntPtr.Zero)
        {
            return null;
        }

        // Marshal from IInspectable to the managed GraphicsCaptureItem
        var item = (GraphicsCaptureItem)Marshal.GetObjectForIUnknown(rawItem);
        Marshal.Release(rawItem);
        return item;
    }

    /// <summary>
    /// Gets a WinRT activation factory and queries it for the specified interface.
    /// Used to obtain IGraphicsCaptureItemInterop from the GraphicsCaptureItem factory.
    /// </summary>
    /// <param name="activatableClassId">Fully qualified WinRT class name.</param>
    /// <param name="iid">GUID of the interface to query for.</param>
    /// <returns>Pointer to the requested interface.</returns>
    private static IntPtr GetActivationFactory(string activatableClassId, Guid iid)
    {
        int hr = RoGetActivationFactory(activatableClassId, ref iid, out IntPtr factory);
        if (hr < 0)
        {
            Marshal.ThrowExceptionForHR(hr);
        }
        return factory;
    }

    // ======================== PIXEL DATA / CROPPING ========================

    /// <summary>
    /// Reads pixel data from a SoftwareBitmap and crops to the specified region.
    /// Handles both HDR (R16G16B16A16Float) and SDR (Bgra8) formats.
    /// </summary>
    /// <param name="bitmap">Source bitmap from screen capture.</param>
    /// <param name="region">Region to crop to (in pixels relative to monitor top-left).</param>
    /// <returns>
    /// Tuple of (croppedPixelData, isHdrFormat).
    /// If HDR: data is R16G16B16A16Float (8 bytes/pixel).
    /// If SDR: data is BGRA8 (4 bytes/pixel).
    /// </returns>
    private static (byte[] pixels, bool isHdr) CropPixelData(SoftwareBitmap bitmap, CaptureRegion region)
    {
        // Determine pixel format and bytes-per-pixel
        bool isHdr = bitmap.BitmapPixelFormat == BitmapPixelFormat.Rgba16
                  || bitmap.BitmapPixelFormat == BitmapPixelFormat.Gray16;

        // For non-standard formats, convert to Bgra8 as fallback
        SoftwareBitmap workBitmap = bitmap;
        int bytesPerPixel;

        if (isHdr)
        {
            // R16G16B16A16 = 8 bytes per pixel
            bytesPerPixel = 8;
        }
        else if (bitmap.BitmapPixelFormat == BitmapPixelFormat.Bgra8)
        {
            bytesPerPixel = 4;
        }
        else
        {
            // Convert unknown formats to Bgra8
            Console.Error.WriteLine(
                $"[capture-hdr] Converting pixel format {bitmap.BitmapPixelFormat} to Bgra8");
            workBitmap = SoftwareBitmap.Convert(bitmap, BitmapPixelFormat.Bgra8, BitmapAlphaMode.Premultiplied);
            bytesPerPixel = 4;
            isHdr = false;
        }

        // Read raw pixel buffer
        int fullWidth = workBitmap.PixelWidth;
        int fullStride = fullWidth * bytesPerPixel;
        byte[] fullPixels = new byte[workBitmap.PixelHeight * fullStride];

        // Use BitmapBuffer for direct pixel access
        using (var buffer = workBitmap.LockBuffer(BitmapBufferAccessMode.Read))
        using (var reference = buffer.CreateReference())
        {
            unsafe
            {
                // Get the raw byte pointer from the IMemoryBufferReference
                ((IMemoryBufferByteAccess)reference).GetBuffer(out byte* dataPtr, out uint capacity);

                // Get the actual stride from the buffer description
                var desc = buffer.GetPlaneDescription(0);
                int actualStride = desc.Stride;

                // Copy row by row to handle stride differences
                for (int y = 0; y < workBitmap.PixelHeight; y++)
                {
                    int srcOffset = y * actualStride;
                    int dstOffset = y * fullStride;
                    int copyBytes = Math.Min(fullStride, actualStride);

                    Marshal.Copy((IntPtr)(dataPtr + srcOffset), fullPixels, dstOffset, copyBytes);
                }
            }
        }

        // If region covers the entire bitmap, skip cropping
        if (region.X == 0 && region.Y == 0 &&
            region.Width == fullWidth && region.Height == workBitmap.PixelHeight)
        {
            if (workBitmap != bitmap) workBitmap.Dispose();
            return (fullPixels, isHdr);
        }

        // Crop to region
        int cropStride = region.Width * bytesPerPixel;
        byte[] croppedPixels = new byte[region.Height * cropStride];

        for (int y = 0; y < region.Height; y++)
        {
            int srcIdx = (region.Y + y) * fullStride + region.X * bytesPerPixel;
            int dstIdx = y * cropStride;
            System.Buffer.BlockCopy(fullPixels, srcIdx, croppedPixels, dstIdx, cropStride);
        }

        if (workBitmap != bitmap) workBitmap.Dispose();
        return (croppedPixels, isHdr);
    }

    // ======================== JPEG ENCODING ========================

    /// <summary>
    /// Encodes 8-bit BGRA pixel data as a JPEG file using Windows.Graphics.Imaging.BitmapEncoder.
    /// </summary>
    /// <param name="bgraPixels">Pixel data in B8G8R8A8 format (4 bytes/pixel).</param>
    /// <param name="width">Image width in pixels.</param>
    /// <param name="height">Image height in pixels.</param>
    /// <param name="quality">JPEG quality (1-100).</param>
    /// <param name="outputPath">File path to write the JPEG to.</param>
    private static async Task EncodeJpegAsync(byte[] bgraPixels, int width, int height, int quality, string outputPath)
    {
        // Ensure output directory exists
        var dir = Path.GetDirectoryName(outputPath);
        if (!string.IsNullOrEmpty(dir))
        {
            Directory.CreateDirectory(dir);
        }

        // Create a SoftwareBitmap from the tone-mapped BGRA pixels
        var sdrBitmap = new SoftwareBitmap(BitmapPixelFormat.Bgra8, width, height, BitmapAlphaMode.Premultiplied);

        // Write pixel data into the bitmap
        using (var buffer = sdrBitmap.LockBuffer(BitmapBufferAccessMode.Write))
        using (var reference = buffer.CreateReference())
        {
            unsafe
            {
                ((IMemoryBufferByteAccess)reference).GetBuffer(out byte* dataPtr, out uint capacity);

                var desc = buffer.GetPlaneDescription(0);
                int dstStride = desc.Stride;
                int srcStride = width * 4;

                // Copy row by row to handle stride differences
                for (int y = 0; y < height; y++)
                {
                    Marshal.Copy(bgraPixels, y * srcStride, (IntPtr)(dataPtr + y * dstStride), srcStride);
                }
            }
        }

        // Encode to JPEG using BitmapEncoder with quality property
        using var fileStream = new FileStream(outputPath, FileMode.Create, FileAccess.Write);
        using var raStream = fileStream.AsRandomAccessStream();

        // Set JPEG quality via encoding options (BitmapEncoder uses 0.0-1.0 range)
        var properties = new BitmapPropertySet
        {
            {
                "ImageQuality",
                new BitmapTypedValue((float)quality / 100.0f, Windows.Foundation.PropertyType.Single)
            }
        };

        var encoder = await BitmapEncoder.CreateAsync(BitmapEncoder.JpegEncoderId, raStream, properties);

        encoder.SetSoftwareBitmap(sdrBitmap);

        await encoder.FlushAsync();

        sdrBitmap.Dispose();

        Console.Error.WriteLine(
            $"[capture-hdr] JPEG encoded: {width}x{height}, quality={quality}");
    }

    // ======================== CLI ARGUMENT PARSING ========================

    /// <summary>
    /// Parses command-line arguments into a <see cref="CaptureOptions"/> struct.
    /// Supports: --monitor, --region, --quality, --output, --help.
    /// </summary>
    /// <param name="args">Raw CLI arguments.</param>
    /// <param name="options">Parsed options (valid only if method returns true).</param>
    /// <returns>True if parsing succeeded, false on error or --help.</returns>
    private static bool TryParseArgs(string[] args, out CaptureOptions options)
    {
        options = new CaptureOptions
        {
            MonitorIndex = 0,
            Quality = DEFAULT_QUALITY,
            Region = null,
            OutputPath = ""
        };

        // Must have at least --output
        if (args.Length == 0)
        {
            PrintUsage();
            return false;
        }

        for (int i = 0; i < args.Length; i++)
        {
            switch (args[i])
            {
                case "--help":
                case "-h":
                    PrintUsage();
                    return false;

                case "--monitor":
                    if (i + 1 >= args.Length)
                    {
                        Console.Error.WriteLine("[capture-hdr] ERROR: --monitor requires a value");
                        return false;
                    }
                    if (!int.TryParse(args[++i], out int monIdx) || monIdx < 0)
                    {
                        Console.Error.WriteLine(
                            $"[capture-hdr] ERROR: Invalid monitor index: {args[i]}");
                        return false;
                    }
                    options.MonitorIndex = monIdx;
                    break;

                case "--region":
                    if (i + 1 >= args.Length)
                    {
                        Console.Error.WriteLine("[capture-hdr] ERROR: --region requires a value (x,y,w,h)");
                        return false;
                    }
                    if (!TryParseRegion(args[++i], out var region))
                    {
                        return false;
                    }
                    options.Region = region;
                    break;

                case "--quality":
                    if (i + 1 >= args.Length)
                    {
                        Console.Error.WriteLine("[capture-hdr] ERROR: --quality requires a value (1-100)");
                        return false;
                    }
                    if (!int.TryParse(args[++i], out int qual) || qual < 1 || qual > 100)
                    {
                        Console.Error.WriteLine(
                            $"[capture-hdr] ERROR: Invalid quality value: {args[i]} (must be 1-100)");
                        return false;
                    }
                    options.Quality = qual;
                    break;

                case "--output":
                    if (i + 1 >= args.Length)
                    {
                        Console.Error.WriteLine("[capture-hdr] ERROR: --output requires a file path");
                        return false;
                    }
                    options.OutputPath = args[++i];
                    break;

                default:
                    Console.Error.WriteLine($"[capture-hdr] ERROR: Unknown argument: {args[i]}");
                    PrintUsage();
                    return false;
            }
        }

        // Validate required arguments
        if (string.IsNullOrWhiteSpace(options.OutputPath))
        {
            Console.Error.WriteLine("[capture-hdr] ERROR: --output is required");
            PrintUsage();
            return false;
        }

        // Validate output path is writable
        try
        {
            var dir = Path.GetDirectoryName(Path.GetFullPath(options.OutputPath));
            if (!string.IsNullOrEmpty(dir) && !Directory.Exists(dir))
            {
                Console.Error.WriteLine(
                    $"[capture-hdr] ERROR: Output directory does not exist: {dir}");
                return false;
            }
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine(
                $"[capture-hdr] ERROR: Invalid output path: {ex.Message}");
            return false;
        }

        Console.Error.WriteLine(
            $"[capture-hdr] Args: monitor={options.MonitorIndex}, " +
            $"region={options.Region?.ToString() ?? "full"}, " +
            $"quality={options.Quality}, output={options.OutputPath}");

        return true;
    }

    /// <summary>
    /// Parses a region string in the format "x,y,w,h" into a <see cref="CaptureRegion"/>.
    /// </summary>
    /// <param name="value">Region string (e.g., "100,200,800,600").</param>
    /// <param name="region">Parsed region (valid only if method returns true).</param>
    /// <returns>True if parsing succeeded.</returns>
    private static bool TryParseRegion(string value, out CaptureRegion region)
    {
        region = default;
        var parts = value.Split(',');

        if (parts.Length != 4)
        {
            Console.Error.WriteLine(
                $"[capture-hdr] ERROR: Region must be x,y,w,h (got {parts.Length} parts): {value}");
            return false;
        }

        if (!int.TryParse(parts[0], out int x) ||
            !int.TryParse(parts[1], out int y) ||
            !int.TryParse(parts[2], out int w) ||
            !int.TryParse(parts[3], out int h))
        {
            Console.Error.WriteLine(
                $"[capture-hdr] ERROR: Region values must be integers: {value}");
            return false;
        }

        // Width and height must be positive
        if (w <= 0 || h <= 0)
        {
            Console.Error.WriteLine(
                $"[capture-hdr] ERROR: Region width and height must be > 0: {value}");
            return false;
        }

        // X and Y must be non-negative
        if (x < 0 || y < 0)
        {
            Console.Error.WriteLine(
                $"[capture-hdr] ERROR: Region x,y must be >= 0: {value}");
            return false;
        }

        region = new CaptureRegion(x, y, w, h);
        return true;
    }

    /// <summary>
    /// Prints CLI usage information to stderr.
    /// </summary>
    private static void PrintUsage()
    {
        Console.Error.WriteLine("Usage: capture-hdr.exe --output <path.jpg> [options]");
        Console.Error.WriteLine();
        Console.Error.WriteLine("Options:");
        Console.Error.WriteLine("  --monitor <index>     Monitor index (default: 0 = primary)");
        Console.Error.WriteLine("  --region <x,y,w,h>    Capture region in pixels (default: full monitor)");
        Console.Error.WriteLine("  --quality <1-100>     JPEG quality (default: 90)");
        Console.Error.WriteLine("  --output <path.jpg>   Output file path (required)");
        Console.Error.WriteLine("  --help, -h            Show this help");
        Console.Error.WriteLine();
        Console.Error.WriteLine("Exit codes:");
        Console.Error.WriteLine("  0  Success");
        Console.Error.WriteLine("  1  Capture failed");
        Console.Error.WriteLine("  2  Encoding failed");
        Console.Error.WriteLine("  3  Invalid arguments");
    }

    // ======================== DISPATCHER QUEUE ========================

    /// <summary>
    /// Creates a DispatcherQueueController on the current thread.
    /// Required by Windows.Graphics.Capture to dispatch frame events.
    /// </summary>
    /// <returns>
    /// Handle to the DispatcherQueueController, or IntPtr.Zero on failure.
    /// </returns>
    private static IntPtr CreateDispatcherQueueController()
    {
        var options = new DispatcherQueueOptions
        {
            dwSize = Marshal.SizeOf<DispatcherQueueOptions>(),
            threadType = DQTYPE_THREAD_CURRENT,  // Use the current thread
            apartmentType = DQTAT_COM_ASTA        // Application STA (for WinRT)
        };

        int hr = CreateDispatcherQueueController(ref options, out IntPtr controller);
        return hr >= 0 ? controller : IntPtr.Zero;
    }

    // ======================== DATA TYPES ========================

    /// <summary>
    /// Parsed CLI options for the capture operation.
    /// </summary>
    private struct CaptureOptions
    {
        /// <summary>Zero-based monitor index (0 = primary).</summary>
        public int MonitorIndex;

        /// <summary>JPEG encoding quality (1-100).</summary>
        public int Quality;

        /// <summary>
        /// Capture region in pixels (relative to monitor top-left).
        /// Null means capture the full monitor.
        /// </summary>
        public CaptureRegion? Region;

        /// <summary>Output file path for the JPEG.</summary>
        public string OutputPath;
    }

    /// <summary>
    /// Defines a rectangular capture region in pixel coordinates.
    /// </summary>
    /// <param name="X">Left offset in pixels.</param>
    /// <param name="Y">Top offset in pixels.</param>
    /// <param name="Width">Width in pixels.</param>
    /// <param name="Height">Height in pixels.</param>
    private record struct CaptureRegion(int X, int Y, int Width, int Height)
    {
        public override string ToString() => $"{X},{Y},{Width}x{Height}";
    }

    /// <summary>
    /// Information about an enumerated display monitor.
    /// </summary>
    private struct MonitorInfo
    {
        public IntPtr Hmonitor;
        public int Left;
        public int Top;
        public int Width;
        public int Height;
        public bool IsPrimary;
    }

    // ======================== COM INTERFACES ========================

    /// <summary>
    /// IGraphicsCaptureItemInterop — COM interface for creating GraphicsCaptureItem
    /// from a Win32 HWND or HMONITOR handle.
    /// </summary>
    [ComImport]
    [Guid("3628E81B-3CAC-4C60-B7F4-23CE0E0C3356")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private interface IGraphicsCaptureItemInterop
    {
        IntPtr CreateForWindow(IntPtr window, ref Guid iid);
        IntPtr CreateForMonitor(IntPtr monitor, ref Guid iid);
    }

    /// <summary>
    /// IMemoryBufferByteAccess — COM interface for direct byte access to IMemoryBuffer.
    /// Used to read/write raw pixel data from SoftwareBitmap buffers.
    /// </summary>
    [ComImport]
    [Guid("5b0d3235-4dba-4d44-865e-8f1d0e4fd04d")]
    [InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    private unsafe interface IMemoryBufferByteAccess
    {
        void GetBuffer(out byte* buffer, out uint capacity);
    }

    // ======================== P/INVOKE DECLARATIONS ========================

    // --- User32: Monitor enumeration ---

    private delegate bool EnumMonitorsDelegate(IntPtr hMonitor, IntPtr hdcMonitor, ref RECT lprcMonitor, IntPtr dwData);

    [DllImport("user32.dll")]
    private static extern bool EnumDisplayMonitors(IntPtr hdc, IntPtr lprcClip, EnumMonitorsDelegate lpfnEnum, IntPtr dwData);

    [DllImport("user32.dll", CharSet = CharSet.Auto)]
    private static extern bool GetMonitorInfo(IntPtr hMonitor, ref MONITORINFOEX lpmi);

    private const uint MONITORINFOF_PRIMARY = 1;

    [StructLayout(LayoutKind.Sequential)]
    private struct RECT
    {
        public int left, top, right, bottom;
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Auto)]
    private struct MONITORINFOEX
    {
        public int cbSize;
        public RECT rcMonitor;
        public RECT rcWork;
        public uint dwFlags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)]
        public string szDevice;
    }

    // --- D3D11: Device creation ---

    private const int D3D_DRIVER_TYPE_HARDWARE = 1;
    private const uint D3D11_CREATE_DEVICE_BGRA_SUPPORT = 0x20;
    private const uint D3D11_SDK_VERSION = 7;

    [DllImport("d3d11.dll")]
    private static extern int D3D11CreateDevice(
        IntPtr pAdapter,
        int driverType,
        IntPtr software,
        uint flags,
        int[]? featureLevels,
        int featureLevelCount,
        uint sdkVersion,
        out IntPtr ppDevice,
        out int pFeatureLevel,
        out IntPtr ppImmediateContext);

    // --- DXGI: WinRT interop ---

    [DllImport("d3d11.dll", EntryPoint = "CreateDirect3D11DeviceFromDXGIDevice",
        SetLastError = true, PreserveSig = true)]
    private static extern int CreateDirect3D11DeviceFromDXGIDevice(
        IntPtr dxgiDevice, out IntPtr graphicsDevice);

    // --- CoreMessaging: DispatcherQueue ---

    private const int DQTYPE_THREAD_CURRENT = 2;
    private const int DQTAT_COM_ASTA = 2;

    [StructLayout(LayoutKind.Sequential)]
    private struct DispatcherQueueOptions
    {
        public int dwSize;
        public int threadType;
        public int apartmentType;
    }

    [DllImport("CoreMessaging.dll")]
    private static extern int CreateDispatcherQueueController(
        ref DispatcherQueueOptions options, out IntPtr dispatcherQueueController);

    // --- WinRT: Activation factory ---

    [DllImport("combase.dll", PreserveSig = true)]
    private static extern int RoGetActivationFactory(
        [MarshalAs(UnmanagedType.HString)] string activatableClassId,
        ref Guid iid,
        out IntPtr factory);
}
