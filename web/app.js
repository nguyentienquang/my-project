// WatchCam Dashboard Client Script
(() => {
  // Authentication & State
  const DEVICE_TOKEN_KEY = 'watchcam_device_token';
  const LEGACY_TOKEN_KEY = 'watchcam_token';

  // Check URL query parameters for token (seamless migration between HTTP and HTTPS)
  let urlToken = null;
  try {
    const urlParams = new URLSearchParams(window.location.search);
    urlToken = urlParams.get('token');
  } catch (_) {}

  let currentToken = urlToken || localStorage.getItem(DEVICE_TOKEN_KEY) || localStorage.getItem(LEGACY_TOKEN_KEY) || '';
  if (urlToken && currentToken) {
    try {
      localStorage.setItem(DEVICE_TOKEN_KEY, currentToken);
      localStorage.setItem(LEGACY_TOKEN_KEY, currentToken);
      // Clean up token parameter from address bar for privacy and clean appearance
      const cleanUrl = window.location.pathname + window.location.hash;
      window.history.replaceState({}, document.title, cleanUrl);
    } catch (_) {}
  } else if (!localStorage.getItem(DEVICE_TOKEN_KEY) && currentToken) {
    try { localStorage.setItem(DEVICE_TOKEN_KEY, currentToken); } catch (_) {}
  }

  function saveDeviceToken(token) {
    currentToken = token;
    try {
      localStorage.setItem(DEVICE_TOKEN_KEY, token);
      localStorage.setItem(LEGACY_TOKEN_KEY, token);
    } catch (_) {}
  }

  function clearDeviceToken() {
    currentToken = '';
    try {
      localStorage.removeItem(DEVICE_TOKEN_KEY);
      localStorage.removeItem(LEGACY_TOKEN_KEY);
    } catch (_) {}
  }
  let ws = null;
  let wsReconnectTimer = null;
  let wsRetryCount = 0;
  let statusPollTimer = null;
  let isSoundEnabled = false;
  let isStreamPaused = false;
  let currentEvents = [];
  let selectedEvent = null;

  // DOM Elements
  const connBadge = document.getElementById('connBadge');
  const connStatusText = document.getElementById('connStatusText');
  const viewerCount = document.getElementById('viewerCount');
  const streamFpsTag = document.getElementById('streamFpsTag');
  const cameraNameTag = document.getElementById('cameraNameTag');
  const streamView = document.getElementById('streamView');
  const detectionOverlay = document.getElementById('detectionOverlay');
  const streamPlaceholder = document.getElementById('streamPlaceholder');
  const placeholderTitle = document.getElementById('placeholderTitle');
  const placeholderDesc = document.getElementById('placeholderDesc');
  const retryStreamBtn = document.getElementById('retryStreamBtn');
  const toggleStreamBtn = document.getElementById('toggleStreamBtn');
  const rotateStreamBtn = document.getElementById('rotateStreamBtn');
  const rotDegTag = document.getElementById('rotDegTag');
  const soundAlertBtn = document.getElementById('soundAlertBtn');
  const btnToggleAudio = document.getElementById('btnToggleAudio');
  const sliderAudioVolume = document.getElementById('sliderAudioVolume');
  const audioStatusBadge = document.getElementById('audioStatusBadge');
  const audioMeterFill = document.getElementById('audioMeterFill');
  const audioMeterValue = document.getElementById('audioMeterValue');
  const btnPushToTalk = document.getElementById('btnPushToTalk');
  const pttText = document.getElementById('pttText');
  const pttStatus = document.getElementById('pttStatus');
  const quickSnapshotBtn = document.getElementById('quickSnapshotBtn');
  const fullscreenBtn = document.getElementById('fullscreenBtn');
  const exitFullscreenTopBtn = document.getElementById('exitFullscreenTopBtn');
  const startCamBtn = document.getElementById('startCamBtn');
  const stopCamBtn = document.getElementById('stopCamBtn');
  const switchCamBtn = document.getElementById('switchCamBtn');
  const streamResolutionSelect = document.getElementById('streamResolutionSelect');
  const streamQualitySelect = document.getElementById('streamQualitySelect');
  const cooldownSelect = document.getElementById('cooldownSelect');
  const motionToggle = document.getElementById('motionToggle');
  const motionScoreVal = document.getElementById('motionScoreVal');
  const motionScoreBar = document.getElementById('motionScoreBar');
  const sensitivityInput = document.getElementById('sensitivityInput');
  const sensitivityVal = document.getElementById('sensitivityVal');
  const latestAlertBanner = document.getElementById('latestAlertBanner');
  const alertTitle = document.getElementById('alertTitle');
  const alertTime = document.getElementById('alertTime');
  const alertQuickViewBtn = document.getElementById('alertQuickViewBtn');
  const eventList = document.getElementById('eventList');
  const eventCountHeader = document.getElementById('eventCountHeader');
  const refreshEventsBtn = document.getElementById('refreshEventsBtn');
  const clearAllEventsBtn = document.getElementById('clearAllEventsBtn');
  const pairModal = document.getElementById('pairModal');
  const pairCodeInput = document.getElementById('pairCodeInput');
  const pairError = document.getElementById('pairError');
  const submitPairBtn = document.getElementById('submitPairBtn');
  const authBtn = document.getElementById('authBtn');
  const logoutBtn = document.getElementById('logoutBtn');
  const snapshotModal = document.getElementById('snapshotModal');
  const closeSnapshotModalBtn = document.getElementById('closeSnapshotModalBtn');
  const modalEventTitle = document.getElementById('modalEventTitle');
  const modalSnapshotImg = document.getElementById('modalSnapshotImg');
  const modalDetectionBox = document.getElementById('modalDetectionBox');
  const modalVideoPlayer = document.getElementById('modalVideoPlayer');
  const modalEventTime = document.getElementById('modalEventTime');
  const modalEventScore = document.getElementById('modalEventScore');
  const modalEventCamera = document.getElementById('modalEventCamera');
  const toggleMediaViewBtn = document.getElementById('toggleMediaViewBtn');
  const downloadSnapshotBtn = document.getElementById('downloadSnapshotBtn');
  const downloadVideoBtn = document.getElementById('downloadVideoBtn');
  const deleteEventBtn = document.getElementById('deleteEventBtn');

  // Microphone Permission Guidance Modal Elements
  const micGuideModal = document.getElementById('micGuideModal');
  const closeMicGuideModalBtn = document.getElementById('closeMicGuideModalBtn');
  const closeMicGuideFooterBtn = document.getElementById('closeMicGuideFooterBtn');
  const btnSwitchToHttps = document.getElementById('btnSwitchToHttps');
  const btnCopyFlagUrl = document.getElementById('btnCopyFlagUrl');
  const btnCopyOriginUrl = document.getElementById('btnCopyOriginUrl');
  const browserFlagUrlInput = document.getElementById('browserFlagUrlInput');
  const cameraOriginInput = document.getElementById('cameraOriginInput');
  const copyFeedbackText = document.getElementById('copyFeedbackText');

  // Email Alert Elements
  const emailAlertsToggle = document.getElementById('emailAlertsToggle');
  const smtpHostInput = document.getElementById('smtpHostInput');
  const smtpPortInput = document.getElementById('smtpPortInput');
  const smtpUsernameInput = document.getElementById('smtpUsernameInput');
  const smtpPasswordInput = document.getElementById('smtpPasswordInput');
  const smtpRecipientInput = document.getElementById('smtpRecipientInput');
  const saveEmailConfigBtn = document.getElementById('saveEmailConfigBtn');
  const testEmailBtn = document.getElementById('testEmailBtn');
  const emailStatusMsg = document.getElementById('emailStatusMsg');

  // Tablet Hardware Health Elements
  const tabletHealthBadge = document.getElementById('tabletHealthBadge');
  const healthBatterySource = document.getElementById('healthBatterySource');
  const healthBatteryPct = document.getElementById('healthBatteryPct');
  const healthBatteryHealth = document.getElementById('healthBatteryHealth');
  const healthBatteryBar = document.getElementById('healthBatteryBar');
  const healthThermalStatus = document.getElementById('healthThermalStatus');
  const healthThermalVal = document.getElementById('healthThermalVal');
  const healthThermalWarning = document.getElementById('healthThermalWarning');
  const healthThermalBar = document.getElementById('healthThermalBar');
  const healthRamPct = document.getElementById('healthRamPct');
  const healthRamVal = document.getElementById('healthRamVal');
  const healthRamAvail = document.getElementById('healthRamAvail');
  const healthRamBar = document.getElementById('healthRamBar');
  const healthStorageFreePct = document.getElementById('healthStorageFreePct');
  const healthStorageVal = document.getElementById('healthStorageVal');
  const healthSnapshotsVal = document.getElementById('healthSnapshotsVal');
  const healthStorageBar = document.getElementById('healthStorageBar');
  const healthWifiQuality = document.getElementById('healthWifiQuality');
  const healthWifiVal = document.getElementById('healthWifiVal');
  const healthWifiSpeed = document.getElementById('healthWifiSpeed');
  const healthWifiSsid = document.getElementById('healthWifiSsid');
  const healthDeviceUptime = document.getElementById('healthDeviceUptime');
  const healthServiceUptime = document.getElementById('healthServiceUptime');

  function formatBytes(bytes) {
    if (bytes == null || isNaN(bytes) || bytes === 0) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let i = 0;
    let val = bytes;
    while (val >= 1024 && i < units.length - 1) {
      val /= 1024;
      i++;
    }
    return `${val.toFixed(1)} ${units[i]}`;
  }

  function formatDuration(seconds) {
    if (seconds == null || isNaN(seconds) || seconds < 0) return '--';
    const d = Math.floor(seconds / 86400);
    const h = Math.floor((seconds % 86400) / 3600);
    const m = Math.floor((seconds % 3600) / 60);
    const s = Math.floor(seconds % 60);
    if (d > 0) return `${d}d ${h}h ${m}m`;
    if (h > 0) return `${h}h ${m}m ${s}s`;
    if (m > 0) return `${m}m ${s}s`;
    return `${s}s`;
  }

  function updateDeviceHealthUI(dev) {
    if (!dev) return;

    // 1. Battery
    if (dev.battery) {
      const bat = dev.battery;
      const level = Math.max(0, Math.min(100, bat.level != null ? bat.level : 0));
      if (healthBatteryPct) healthBatteryPct.textContent = `${level}%`;
      if (healthBatteryBar) healthBatteryBar.style.width = `${level}%`;
      if (healthBatterySource) healthBatterySource.textContent = bat.chargingSource || (bat.isCharging ? 'Đang sạc' : 'Pin');
      if (healthBatteryHealth) healthBatteryHealth.textContent = bat.health || 'Tốt';

      if (healthBatteryBar) {
        if (level <= 15) {
          healthBatteryBar.style.backgroundColor = 'var(--accent-red)';
        } else if (level <= 30) {
          healthBatteryBar.style.backgroundColor = 'var(--accent-amber)';
        } else {
          healthBatteryBar.style.backgroundColor = 'var(--accent-green)';
        }
      }
    }

    // 2. Thermal
    const temp = dev.thermal?.temperature ?? dev.battery?.temperature;
    const thermalStatus = dev.thermal?.status || 'Bình thường';
    if (temp != null) {
      if (healthThermalVal) healthThermalVal.textContent = `${Number(temp).toFixed(1)} °C`;
      if (healthThermalStatus) healthThermalStatus.textContent = thermalStatus;

      // Scale temp 20°C - 60°C to 0 - 100%
      const tempPct = Math.max(0, Math.min(100, ((temp - 20) / 40) * 100));
      if (healthThermalBar) healthThermalBar.style.width = `${tempPct}%`;

      if (temp >= 45.0 || thermalStatus === 'Quá nhiệt') {
        if (healthThermalStatus) healthThermalStatus.className = 'metric-badge hot';
        if (healthThermalBar) healthThermalBar.style.backgroundColor = 'var(--accent-red)';
        if (healthThermalWarning) healthThermalWarning.style.display = 'inline';
        if (tabletHealthBadge) {
          tabletHealthBadge.className = 'health-badge-pill hot';
          tabletHealthBadge.textContent = 'Quá nhiệt';
        }
      } else if (temp >= 38.0 || thermalStatus === 'Ấm') {
        if (healthThermalStatus) healthThermalStatus.className = 'metric-badge warm';
        if (healthThermalBar) healthThermalBar.style.backgroundColor = 'var(--accent-amber)';
        if (healthThermalWarning) healthThermalWarning.style.display = 'none';
        if (tabletHealthBadge) {
          tabletHealthBadge.className = 'health-badge-pill warm';
          tabletHealthBadge.textContent = 'Ấm';
        }
      } else {
        if (healthThermalStatus) healthThermalStatus.className = 'metric-badge normal';
        if (healthThermalBar) healthThermalBar.style.backgroundColor = 'var(--accent-green)';
        if (healthThermalWarning) healthThermalWarning.style.display = 'none';
        if (tabletHealthBadge) {
          tabletHealthBadge.className = 'health-badge-pill normal';
          tabletHealthBadge.textContent = 'Bình thường';
        }
      }
    }

    // 3. RAM
    if (dev.ram) {
      const ram = dev.ram;
      const usedPct = Math.max(0, Math.min(100, ram.usedPercent != null ? ram.usedPercent : 0));
      if (healthRamPct) healthRamPct.textContent = `${usedPct}%`;
      if (healthRamBar) healthRamBar.style.width = `${usedPct}%`;
      if (healthRamVal) healthRamVal.textContent = `${formatBytes(ram.usedBytes)} / ${formatBytes(ram.totalBytes)}`;
      if (healthRamAvail && ram.availableBytes != null) {
        healthRamAvail.textContent = `Trống: ${formatBytes(ram.availableBytes)}`;
      }

      if (healthRamBar) {
        if (usedPct >= 90) {
          healthRamBar.style.backgroundColor = 'var(--accent-red)';
        } else if (usedPct >= 75) {
          healthRamBar.style.backgroundColor = 'var(--accent-amber)';
        } else {
          healthRamBar.style.backgroundColor = 'var(--accent-blue)';
        }
      }
    }

    // 4. Storage
    if (dev.storage) {
      const stor = dev.storage;
      const total = Math.max(stor.totalBytes || 1, stor.freeBytes || 0);
      const free = Math.min(stor.freeBytes || 0, total);
      const used = Math.max(0, total - free);
      const usedPct = Math.max(0, Math.min(100, Math.round((used / total) * 100)));
      const freePct = 100 - usedPct;

      if (healthStorageFreePct) healthStorageFreePct.textContent = `Trống ${freePct}%`;
      if (healthStorageBar) healthStorageBar.style.width = `${usedPct}%`;
      if (healthStorageVal) healthStorageVal.textContent = `${formatBytes(free)} / ${formatBytes(total)}`;
      if (healthSnapshotsVal) healthSnapshotsVal.textContent = `Ảnh & Video: ${formatBytes(stor.snapshotsBytes || 0)}`;

      if (healthStorageBar) {
        if (freePct <= 10) {
          healthStorageBar.style.backgroundColor = 'var(--accent-red)';
        } else {
          healthStorageBar.style.backgroundColor = '#a371f7';
        }
      }
    }

    // 5. Wi-Fi
    if (dev.wifi) {
      const wifi = dev.wifi;
      if (healthWifiVal) healthWifiVal.textContent = `${wifi.rssi != null ? wifi.rssi : 0} dBm`;
      if (healthWifiSpeed) healthWifiSpeed.textContent = `${wifi.linkSpeedMbps || 0} Mbps`;
      if (healthWifiQuality) healthWifiQuality.textContent = wifi.signalQuality || (wifi.connected ? 'Tốt' : 'Mất sóng');
      if (healthWifiSsid) healthWifiSsid.textContent = wifi.ssid ? `Mạng: ${wifi.ssid}` : (wifi.connected ? 'Đã kết nối Wi-Fi' : 'Không có Wi-Fi');

      if (healthWifiQuality) {
        if (!wifi.connected || wifi.signalQuality === 'Không kết nối') {
          healthWifiQuality.className = 'metric-badge hot';
        } else if (wifi.signalQuality === 'Yếu') {
          healthWifiQuality.className = 'metric-badge warm';
        } else {
          healthWifiQuality.className = 'metric-badge normal';
        }
      }
    }

    // 6. Uptime
    if (dev.uptime) {
      if (healthDeviceUptime) healthDeviceUptime.textContent = formatDuration(dev.uptime.deviceUptimeSeconds);
      if (healthServiceUptime) healthServiceUptime.textContent = formatDuration(dev.uptime.serviceUptimeSeconds);
    }
  }

  // Video Rotation State
  let currentRotation = parseInt(localStorage.getItem('watchcam_rotation') || '0', 10);
  if (![0, 90, 180, 270].includes(currentRotation)) currentRotation = 0;
  let activeDetectionData = null;
  let lastRenderedRotation = currentRotation;

  function applyRotation(deg) {
    currentRotation = deg;
    localStorage.setItem('watchcam_rotation', deg.toString());
    if (rotDegTag) rotDegTag.textContent = `${deg}°`;

    document.querySelectorAll('.rot-btn').forEach(btn => {
      const bDeg = parseInt(btn.getAttribute('data-deg'), 10);
      btn.classList.toggle('active', bDeg === deg);
    });

    const container = document.getElementById('videoContainer');
    if (deg === 90 || deg === 270) {
      const cw = container ? (container.clientWidth || 640) : 640;
      const ch = container ? (container.clientHeight || 480) : 480;
      const scale = Math.min(cw / (ch || 1), ch / (cw || 1));
      streamView.style.transform = `rotate(${deg}deg) scale(${scale})`;
    } else {
      streamView.style.transform = `rotate(${deg}deg) scale(1)`;
    }

    if (activeDetectionData && detectionBoxElement && !detectionBoxElement.classList.contains('fading')) {
      renderDetectionBox(activeDetectionData.boundingBox, activeDetectionData.entityType, activeDetectionData.confidence);
    }
  }

  window.addEventListener('resize', () => {
    applyRotation(currentRotation);
  });

  window.addEventListener('orientationchange', () => {
    setTimeout(() => applyRotation(currentRotation), 150);
  });

  if (window.screen && window.screen.orientation && typeof window.screen.orientation.addEventListener === 'function') {
    window.screen.orientation.addEventListener('change', () => {
      setTimeout(() => applyRotation(currentRotation), 150);
    });
  }

  if (rotateStreamBtn) {
    rotateStreamBtn.addEventListener('click', () => {
      const nextDeg = (currentRotation + 90) % 360;
      applyRotation(nextDeg);
    });
  }

  document.querySelectorAll('.rot-btn').forEach(btn => {
    btn.addEventListener('click', () => {
      const deg = parseInt(btn.getAttribute('data-deg'), 10);
      applyRotation(deg);
    });
  });

  // Web Audio Context for Beep Alert
  let audioCtx = null;
  function playAlertBeep() {
    if (!isSoundEnabled) return;
    try {
      if (!audioCtx) {
        audioCtx = new (window.AudioContext || window.webkitAudioContext)();
      }
      if (audioCtx.state === 'suspended') {
        audioCtx.resume();
      }
      const osc1 = audioCtx.createOscillator();
      const osc2 = audioCtx.createOscillator();
      const gain = audioCtx.createGain();

      osc1.type = 'sine';
      osc1.frequency.setValueAtTime(880, audioCtx.currentTime); // A5
      osc1.frequency.exponentialRampToValueAtTime(1320, audioCtx.currentTime + 0.15); // E6

      osc2.type = 'sine';
      osc2.frequency.setValueAtTime(1760, audioCtx.currentTime + 0.18);
      osc2.frequency.exponentialRampToValueAtTime(880, audioCtx.currentTime + 0.35);

      gain.gain.setValueAtTime(0.2, audioCtx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.01, audioCtx.currentTime + 0.4);

      osc1.connect(gain);
      osc2.connect(gain);
      gain.connect(audioCtx.destination);

      osc1.start(audioCtx.currentTime);
      osc1.stop(audioCtx.currentTime + 0.15);
      osc2.start(audioCtx.currentTime + 0.18);
      osc2.stop(audioCtx.currentTime + 0.4);
    } catch (e) {
      console.warn('Audio alert error:', e);
    }
  }

  // Network helpers with auth error handling
  async function apiFetch(url, options = {}) {
    options.credentials = 'same-origin';
    options.headers = options.headers || {};
    if (currentToken && !options.headers['Authorization']) {
      options.headers['Authorization'] = `Bearer ${currentToken}`;
    }

    try {
      const res = await fetch(url, options);
      if (res.status === 401) {
        if (currentToken) {
          clearDeviceToken();
          clearTimeout(wsReconnectTimer);
          if (ws) {
            const oldWs = ws;
            ws = null;
            oldWs.close();
          }
          streamView.src = '';
          showPairModal(true);
        }
        throw new Error('Yêu cầu xác thực ghép nối');
      }
      return res;
    } catch (err) {
      throw err;
    }
  }

  // Stream Management
  function initStream() {
    if (isStreamPaused) return;
    streamPlaceholder.style.display = 'none';
    streamView.style.display = 'block';
    const tokenQuery = currentToken ? `&token=${encodeURIComponent(currentToken)}` : '';
    streamView.src = `/stream.mjpeg?t=${Date.now()}${tokenQuery}`;
  }

  streamView.onerror = () => {
    if (!isStreamPaused) {
      streamView.style.display = 'none';
      streamPlaceholder.style.display = 'flex';
      placeholderTitle.textContent = 'Mất kết nối luồng video';
      placeholderDesc.textContent = 'Không nhận được dữ liệu từ camera. Đang thử kết nối lại...';
    }
  };

  streamView.addEventListener('load', () => {
    if (activeDetectionData && detectionBoxElement && !detectionBoxElement.classList.contains('fading')) {
      renderDetectionBox(activeDetectionData.boundingBox, activeDetectionData.entityType, activeDetectionData.confidence);
    }
  });

  retryStreamBtn.addEventListener('click', () => {
    initStream();
  });

  toggleStreamBtn.addEventListener('click', () => {
    isStreamPaused = !isStreamPaused;
    if (isStreamPaused) {
      streamView.src = '';
      streamView.style.display = 'none';
      streamPlaceholder.style.display = 'flex';
      placeholderTitle.textContent = 'Luồng đã tạm dừng';
      placeholderDesc.textContent = 'Nhấn nút tiếp tục để xem luồng trực tiếp.';
      toggleStreamBtn.textContent = 'Tiếp tục';
    } else {
      toggleStreamBtn.textContent = 'Tạm dừng';
      initStream();
    }
  });

  soundAlertBtn.addEventListener('click', () => {
    isSoundEnabled = !isSoundEnabled;
    soundAlertBtn.textContent = isSoundEnabled ? 'Tắt chuông' : 'Bật chuông';
    soundAlertBtn.style.color = isSoundEnabled ? '#3fb950' : '#8b949e';
    if (isSoundEnabled) {
      playAlertBeep();
    }
  });

  // ==========================================================================
  // Real-Time Audio Streaming (Web Audio API) & Two-Way Intercom (Push-to-Talk)
  // ==========================================================================
  let audioContext = null;
  let audioGainNode = null;
  let audioWs = null;
  let isAudioActive = false;
  let audioNextPlayTime = 0;
  let micMediaStream = null;
  let micAudioContext = null;
  let micProcessorNode = null;
  let isTalking = false;
  let pttPendingChunks = [];
  let detectionBoxElement = null;
  let detectionTagElement = null;
  let detectionFadeTimeout = null;
  let detectionHideTimeout = null;
  let lastBoxCenterX = null;
  let lastBoxCenterY = null;
  let currentHttpsPort = 8443;
  let copyFeedbackTimeout = null;

  function renderDetectionBox(boundingBox, entityType, confidence) {
    if (!detectionOverlay) return;
    if (!boundingBox || boundingBox.length < 4 || (boundingBox[2] <= 0 && boundingBox[3] <= 0)) {
      activeDetectionData = null;
      if (detectionBoxElement) {
        detectionBoxElement.classList.add('fading');
        clearTimeout(detectionHideTimeout);
        detectionHideTimeout = setTimeout(() => {
          if (detectionBoxElement && detectionBoxElement.classList.contains('fading')) {
            detectionOverlay.style.display = 'none';
            lastBoxCenterX = null;
            lastBoxCenterY = null;
            activeDetectionData = null;
          }
        }, 450);
      }
      return;
    }

    const [bx, by, bw, bh] = boundingBox;
    const entity = entityType || 'motion';
    const confPct = Math.round((confidence || 0) * 100);

    activeDetectionData = {
      boundingBox,
      entityType: entity,
      confidence
    };

    const labelVi = entity === 'person'
      ? `Người (${confPct}%)`
      : (entity === 'object' ? `Vật thể (${confPct}%)` : `Chuyển động (${confPct}%)`);

    // Compute exact rendered bounds of the video image within video container
    // Accounts for letterboxing/pillarboxing caused by 16:9 container vs 4:3 camera frame
    const vContainer = document.getElementById('videoContainer');
    const cW = (vContainer && vContainer.clientWidth) || detectionOverlay.clientWidth || 640;
    const cH = (vContainer && vContainer.clientHeight) || detectionOverlay.clientHeight || 360;
    const imgW = (streamView && streamView.naturalWidth > 0) ? streamView.naturalWidth : 640;
    const imgH = (streamView && streamView.naturalHeight > 0) ? streamView.naturalHeight : 480;

    const containerRatio = cW / (cH || 1);
    const imgRatio = imgW / (imgH || 1);

    let renderW = cW;
    let renderH = cH;
    let offsetX = 0;
    let offsetY = 0;

    if (containerRatio > imgRatio) {
      // Pillarbox (black bars on left and right)
      renderH = cH;
      renderW = cH * imgRatio;
      offsetX = (cW - renderW) / 2;
    } else if (containerRatio < imgRatio) {
      // Letterbox (black bars on top and bottom)
      renderW = cW;
      renderH = cW / imgRatio;
      offsetY = (cH - renderH) / 2;
    }

    const deg = currentRotation || 0;
    const scale = (deg === 90 || deg === 270) ? Math.min(cW / (cH || 1), cH / (cW || 1)) : 1.0;

    const centerX = cW / 2;
    const centerY = cH / 2;

    const duMin = (bx - 0.5) * renderW;
    const duMax = (bx + bw - 0.5) * renderW;
    const dvMin = (by - 0.5) * renderH;
    const dvMax = (by + bh - 0.5) * renderH;

    let leftPx;
    let topPx;
    let widthPx;
    let heightPx;

    switch (deg) {
      case 90:
        leftPx = centerX - scale * dvMax;
        topPx = centerY + scale * duMin;
        widthPx = scale * (dvMax - dvMin);
        heightPx = scale * (duMax - duMin);
        break;
      case 180:
        leftPx = centerX - duMax;
        topPx = centerY - dvMax;
        widthPx = duMax - duMin;
        heightPx = dvMax - dvMin;
        break;
      case 270:
        leftPx = centerX + scale * dvMin;
        topPx = centerY - scale * duMax;
        widthPx = scale * (dvMax - dvMin);
        heightPx = scale * (duMax - duMin);
        break;
      case 0:
      default:
        leftPx = centerX + duMin;
        topPx = centerY + dvMin;
        widthPx = duMax - duMin;
        heightPx = dvMax - dvMin;
        break;
    }

    leftPx = Math.max(0, Math.min(cW - 2, leftPx));
    topPx = Math.max(0, Math.min(cH - 2, topPx));
    widthPx = Math.max(4, Math.min(cW - leftPx, widthPx));
    heightPx = Math.max(4, Math.min(cH - topPx, heightPx));

    const leftPct = ((leftPx / cW) * 100).toFixed(2);
    const topPct = ((topPx / cH) * 100).toFixed(2);
    const widthPct = ((widthPx / cW) * 100).toFixed(2);
    const heightPct = ((heightPx / cH) * 100).toFixed(2);

    clearTimeout(detectionFadeTimeout);
    clearTimeout(detectionHideTimeout);

    const isNewlyCreated = !detectionBoxElement || !detectionOverlay.contains(detectionBoxElement);
    if (isNewlyCreated) {
      detectionOverlay.innerHTML = '';
      detectionBoxElement = document.createElement('div');
      detectionTagElement = document.createElement('span');
      detectionTagElement.className = 'detection-tag';
      detectionBoxElement.appendChild(detectionTagElement);
      detectionOverlay.appendChild(detectionBoxElement);
    }

    const wasHidden = isNewlyCreated || detectionOverlay.style.display === 'none' || detectionBoxElement.classList.contains('fading');

    // Check if the target position jumped significantly (e.g. > 40% screen distance) or rotation changed
    const currentCenterX = (leftPx + widthPx * 0.5) / cW;
    const currentCenterY = (topPx + heightPx * 0.5) / cH;
    const isDiscontinuousJump = lastBoxCenterX != null && (
      lastRenderedRotation !== deg ||
      Math.hypot(currentCenterX - lastBoxCenterX, currentCenterY - lastBoxCenterY) > 0.40
    );
    lastRenderedRotation = deg;

    if (wasHidden || isDiscontinuousJump) {
      // Snap to position without transition to avoid ghost bounding box flying across screen
      detectionBoxElement.style.transition = 'none';
      detectionBoxElement.style.left = `${leftPct}%`;
      detectionBoxElement.style.top = `${topPct}%`;
      detectionBoxElement.style.width = `${widthPct}%`;
      detectionBoxElement.style.height = `${heightPct}%`;
      // Force layout reflow so the instant position is committed before re-enabling transitions
      void detectionBoxElement.offsetHeight;
      detectionBoxElement.style.transition = '';
    } else {
      detectionBoxElement.style.left = `${leftPct}%`;
      detectionBoxElement.style.top = `${topPct}%`;
      detectionBoxElement.style.width = `${widthPct}%`;
      detectionBoxElement.style.height = `${heightPct}%`;
    }

    lastBoxCenterX = currentCenterX;
    lastBoxCenterY = currentCenterY;

    detectionBoxElement.className = `detection-box ${entity}`;
    detectionBoxElement.classList.remove('fading');
    detectionTagElement.textContent = labelVi;

    // Keep tag inside container if box is close to top border
    if (topPx < 28) {
      detectionTagElement.style.top = '2px';
    } else {
      detectionTagElement.style.top = '-24px';
    }

    detectionOverlay.style.display = 'block';

    detectionFadeTimeout = setTimeout(() => {
      if (detectionBoxElement) {
        detectionBoxElement.classList.add('fading');
        detectionHideTimeout = setTimeout(() => {
          if (detectionBoxElement && detectionBoxElement.classList.contains('fading')) {
            detectionOverlay.style.display = 'none';
            lastBoxCenterX = null;
            lastBoxCenterY = null;
            activeDetectionData = null;
          }
        }, 450);
      }
    }, 2800);
  }

  function initAudioContext() {
    if (!audioContext) {
      const AudioCtx = window.AudioContext || window.webkitAudioContext;
      try {
        audioContext = new AudioCtx({ sampleRate: 16000 });
      } catch (_) {
        audioContext = new AudioCtx();
      }
      audioGainNode = audioContext.createGain();
      const initialVol = sliderAudioVolume ? sliderAudioVolume.value / 100 : 0.8;
      audioGainNode.gain.value = initialVol;
      audioGainNode.connect(audioContext.destination);
    }
    if (audioContext.state === 'suspended') {
      audioContext.resume().catch(() => {});
    }
  }

  function updateAudioVuMeter(level) {
    if (!audioMeterFill) return;
    const clamped = Math.max(0, Math.min(100, level));
    audioMeterFill.style.width = `${clamped}%`;
    if (audioMeterValue) {
      audioMeterValue.textContent = `${clamped}%`;
    }
  }

  function startLiveAudio() {
    initAudioContext();
    isAudioActive = true;
    audioNextPlayTime = 0;
    if (btnToggleAudio) {
      btnToggleAudio.textContent = 'Tắt tiếng';
      btnToggleAudio.title = 'Tắt âm thanh trực tiếp';
      btnToggleAudio.style.color = '#3fb950';
    }
    if (audioStatusBadge) {
      audioStatusBadge.textContent = 'Đang phát âm thanh';
      audioStatusBadge.className = 'badge-status active';
    }
    connectAudioWs();
  }

  function stopLiveAudio() {
    isAudioActive = false;
    if (btnToggleAudio) {
      btnToggleAudio.textContent = 'Bật tiếng';
      btnToggleAudio.title = 'Bật âm thanh trực tiếp';
      btnToggleAudio.style.color = '';
    }
    if (audioStatusBadge) {
      audioStatusBadge.textContent = 'Đã tắt tiếng';
      audioStatusBadge.className = 'badge-status muted';
    }
    updateAudioVuMeter(0);
    if (audioWs && !isTalking) {
      try { audioWs.close(); } catch (_) {}
      audioWs = null;
    }
  }

  function toggleLiveAudio() {
    if (isAudioActive) {
      stopLiveAudio();
    } else {
      startLiveAudio();
    }
  }

  function connectAudioWs() {
    if (audioWs && (audioWs.readyState === WebSocket.OPEN || audioWs.readyState === WebSocket.CONNECTING)) {
      return;
    }

    const loc = window.location;
    const proto = loc.protocol === 'https:' ? 'wss:' : 'ws:';
    const tokenParam = currentToken ? `?token=${encodeURIComponent(currentToken)}` : '';
    const wsUrl = `${proto}//${loc.host}/ws/audio${tokenParam}`;

    try {
      audioWs = new WebSocket(wsUrl);
      audioWs.binaryType = 'arraybuffer';

      audioWs.onopen = () => {
        console.log('[AudioWS] Connected to live audio stream');
        // Flush any chunks recorded during push-to-talk connection handshake
        while (pttPendingChunks.length > 0) {
          const chunk = pttPendingChunks.shift();
          try {
            audioWs.send(chunk);
          } catch (_) {}
        }
      };

      audioWs.onmessage = (event) => {
        if (!isAudioActive) return;
        const arrayBuffer = event.data;
        if (!(arrayBuffer instanceof ArrayBuffer)) return;

        playPcmChunk(arrayBuffer);
      };

      audioWs.onclose = () => {
        console.log('[AudioWS] Connection closed');
        audioWs = null;
        if (isAudioActive) {
          setTimeout(() => {
            if (isAudioActive) connectAudioWs();
          }, 2000);
        }
      };

      audioWs.onerror = (err) => {
        console.warn('[AudioWS] Error', err);
      };
    } catch (e) {
      console.error('[AudioWS] Failed to connect', e);
    }
  }

  function playPcmChunk(arrayBuffer) {
    if (!audioContext || !audioGainNode) return;
    const int16 = new Int16Array(arrayBuffer);
    if (int16.length === 0) return;

    let sumSquares = 0;
    const float32 = new Float32Array(int16.length);
    for (let i = 0; i < int16.length; i++) {
      const sample = int16[i] / 32768.0;
      float32[i] = sample;
      sumSquares += sample * sample;
    }

    const rms = Math.sqrt(sumSquares / int16.length);
    // Multiply by 250 for dynamic VU meter visual responsiveness
    updateAudioVuMeter(Math.min(100, Math.round(rms * 250)));

    try {
      const buffer = audioContext.createBuffer(1, float32.length, 16000);
      buffer.getChannelData(0).set(float32);

      const source = audioContext.createBufferSource();
      source.buffer = buffer;
      source.connect(audioGainNode);

      const now = audioContext.currentTime;
      // Clamp jitter buffer between now and now + 350ms to absorb Wi-Fi network jitter without dropouts
      if (audioNextPlayTime < now || audioNextPlayTime > now + 0.35) {
        audioNextPlayTime = now + 0.12; // 120ms jitter buffer for smooth continuous playback
      }
      source.start(audioNextPlayTime);
      audioNextPlayTime += buffer.duration;
    } catch (e) {
      console.warn('[AudioPlayer] Error scheduling buffer', e);
    }
  }

  // Linear interpolation resampling to exactly 16000Hz for tablet speaker compatibility
  function resampleTo16k(inputData, inputSampleRate) {
    if (!inputData || inputData.length === 0 || inputSampleRate === 16000) {
      return inputData;
    }
    const ratio = inputSampleRate / 16000;
    const newLength = Math.round(inputData.length / ratio);
    const result = new Float32Array(newLength);
    for (let i = 0; i < newLength; i++) {
      const origIndex = i * ratio;
      const indexFloor = Math.floor(origIndex);
      const indexCeil = Math.min(inputData.length - 1, Math.ceil(origIndex));
      const fraction = origIndex - indexFloor;
      result[i] = inputData[indexFloor] * (1 - fraction) + inputData[indexCeil] * fraction;
    }
    return result;
  }

  // Microphone Permission Guidance & Secure Context Utilities
  function isSecureOriginForMicrophone() {
    if (window.isSecureContext === true) return true;
    if (window.location.protocol === 'https:') return true;
    const hostname = window.location.hostname;
    if (hostname === 'localhost' || hostname === '127.0.0.1' || hostname === '::1') return true;
    return false;
  }

  function getBrowserFlagUrl() {
    const ua = navigator.userAgent || '';
    const isBrave = !!(navigator.brave && typeof navigator.brave.isBrave === 'function') || ua.includes('Brave');
    if (isBrave) {
      return 'brave://flags/#unsafely-treat-insecure-origin-as-secure';
    }
    if (ua.includes('Edg/')) {
      return 'edge://flags/#unsafely-treat-insecure-origin-as-secure';
    }
    if (ua.includes('OPR/') || ua.includes('Opera')) {
      return 'opera://flags/#unsafely-treat-insecure-origin-as-secure';
    }
    return 'chrome://flags/#unsafely-treat-insecure-origin-as-secure';
  }

  function openMicGuideModal() {
    if (!micGuideModal) return;
    const flagUrl = getBrowserFlagUrl();
    if (browserFlagUrlInput) {
      browserFlagUrlInput.value = flagUrl;
    }
    const currentOrigin = window.location.origin || `${window.location.protocol}//${window.location.host}`;
    if (cameraOriginInput) {
      cameraOriginInput.value = currentOrigin;
    }
    if (btnSwitchToHttps) {
      const port = currentHttpsPort || 8443;
      const tokenParam = currentToken ? `?token=${encodeURIComponent(currentToken)}` : '';
      btnSwitchToHttps.href = `https://${window.location.hostname}:${port}/${tokenParam}`;
    }
    if (copyFeedbackText) {
      copyFeedbackText.style.display = 'none';
    }
    micGuideModal.style.display = 'flex';
  }

  function closeMicGuideModal() {
    if (!micGuideModal) return;
    micGuideModal.style.display = 'none';
    if (copyFeedbackTimeout) {
      clearTimeout(copyFeedbackTimeout);
      copyFeedbackTimeout = null;
    }
  }

  function showCopyFeedbackMessage(msg) {
    if (!copyFeedbackText) return;
    copyFeedbackText.textContent = msg || 'Đã sao chép vào bộ nhớ tạm';
    copyFeedbackText.style.display = 'inline-block';
    if (copyFeedbackTimeout) {
      clearTimeout(copyFeedbackTimeout);
    }
    copyFeedbackTimeout = setTimeout(() => {
      copyFeedbackText.style.display = 'none';
    }, 3000);
  }

  function copyTextToClipboard(text) {
    if (!text) return;
    if (navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
      navigator.clipboard.writeText(text)
        .then(() => showCopyFeedbackMessage('Đã sao chép vào bộ nhớ tạm'))
        .catch(() => fallbackCopyText(text));
    } else {
      fallbackCopyText(text);
    }
  }

  function fallbackCopyText(text) {
    try {
      const ta = document.createElement('textarea');
      ta.value = text;
      ta.style.position = 'fixed';
      ta.style.left = '-9999px';
      ta.style.top = '0';
      ta.setAttribute('readonly', '');
      document.body.appendChild(ta);
      ta.focus();
      ta.select();
      ta.setSelectionRange(0, 999999);
      const success = document.execCommand('copy');
      document.body.removeChild(ta);
      if (success) {
        showCopyFeedbackMessage('Đã sao chép vào bộ nhớ tạm');
      } else {
        showCopyFeedbackMessage('Vui lòng chọn và sao chép thủ công');
      }
    } catch (_) {
      showCopyFeedbackMessage('Vui lòng chọn và sao chép thủ công');
    }
  }

  // Cross-browser compatibility wrapper for microphone access
  function getUserMediaCompat(constraints) {
    // 1. Modern standard API
    if (navigator.mediaDevices && typeof navigator.mediaDevices.getUserMedia === 'function') {
      return navigator.mediaDevices.getUserMedia(constraints);
    }

    // 2. Legacy vendor prefix fallback
    const legacyGetUserMedia = navigator.getUserMedia ||
      navigator.webkitGetUserMedia ||
      navigator.mozGetUserMedia ||
      navigator.msGetUserMedia;

    if (legacyGetUserMedia) {
      return new Promise((resolve, reject) => {
        legacyGetUserMedia.call(navigator, constraints, resolve, reject);
      });
    }

    // 3. Not available - inspect security context
    const isSecure = isSecureOriginForMicrophone();
    if (!isSecure) {
      const err = new Error('Microphone bị chặn do trình duyệt yêu cầu kết nối an toàn HTTPS hoặc localhost');
      err.name = 'InsecureContextError';
      return Promise.reject(err);
    }

    const err = new Error('Trình duyệt không hỗ trợ giao diện thu âm micro');
    err.name = 'NotSupportedError';
    return Promise.reject(err);
  }

  // Push-To-Talk (Two-Way Intercom)
  async function startPushToTalk() {
    if (isTalking) return;

    // Upfront check: if on insecure remote origin without mediaDevices, guide the user immediately!
    const isSecure = isSecureOriginForMicrophone();
    const hasMediaDevices = !!(navigator.mediaDevices && typeof navigator.mediaDevices.getUserMedia === 'function');
    const hasLegacyMedia = !!(navigator.getUserMedia || navigator.webkitGetUserMedia || navigator.mozGetUserMedia || navigator.msGetUserMedia);

    if (!isSecure && !hasMediaDevices && !hasLegacyMedia) {
      openMicGuideModal();
      return;
    }

    isTalking = true;

    // Mobile Safari & Chrome critical fix: Synchronously create/resume AudioContext on user gesture!
    const AudioCtx = window.AudioContext || window.webkitAudioContext;
    if (!micAudioContext) {
      try {
        micAudioContext = new AudioCtx();
      } catch (_) {}
    }
    if (micAudioContext && micAudioContext.state === 'suspended') {
      micAudioContext.resume().catch(() => {});
    }

    if (btnPushToTalk) {
      btnPushToTalk.classList.add('talking');
    }
    if (pttText) {
      pttText.textContent = 'Đang truyền giọng nói...';
    }
    if (pttStatus) {
      pttStatus.textContent = 'Đang nói... (Thả nút để dừng)';
      pttStatus.style.color = '#f85149';
    }

    connectAudioWs();

    try {
      let stream = null;
      try {
        stream = await getUserMediaCompat({
          audio: {
            sampleRate: { ideal: 16000 },
            channelCount: 1,
            echoCancellation: true,
            noiseSuppression: true,
            autoGainControl: true,
          }
        });
      } catch (constraintErr) {
        if (constraintErr && constraintErr.name === 'InsecureContextError') {
          throw constraintErr;
        }
        console.warn('[Intercom] Constrained getUserMedia failed, falling back to basic audio', constraintErr);
        stream = await getUserMediaCompat({ audio: true });
      }

      // If user released the button while waiting for microphone permission, clean up immediately
      if (!isTalking) {
        stream.getTracks().forEach(t => t.stop());
        return;
      }
      micMediaStream = stream;

      if (!micAudioContext) {
        micAudioContext = new AudioCtx();
      }
      if (micAudioContext.state === 'suspended') {
        await micAudioContext.resume();
      }

      const actualSampleRate = micAudioContext.sampleRate;
      const sourceNode = micAudioContext.createMediaStreamSource(micMediaStream);

      micProcessorNode = micAudioContext.createScriptProcessor(2048, 1, 1);
      micProcessorNode.onaudioprocess = (e) => {
        if (!isTalking) return;
        const rawInput = e.inputBuffer.getChannelData(0);
        const resampled = resampleTo16k(rawInput, actualSampleRate);
        const pcm16 = new Int16Array(resampled.length);
        for (let i = 0; i < resampled.length; i++) {
          const s = Math.max(-1, Math.min(1, resampled[i]));
          pcm16[i] = s < 0 ? s * 0x8000 : s * 0x7FFF;
        }

        if (audioWs && audioWs.readyState === WebSocket.OPEN) {
          audioWs.send(pcm16.buffer);
        } else if (audioWs && audioWs.readyState === WebSocket.CONNECTING) {
          if (pttPendingChunks.length < 25) {
            pttPendingChunks.push(pcm16.buffer);
          }
        }
      };

      sourceNode.connect(micProcessorNode);
      const dummyGain = micAudioContext.createGain();
      dummyGain.gain.value = 0;
      micProcessorNode.connect(dummyGain);
      dummyGain.connect(micAudioContext.destination);

    } catch (err) {
      console.error('[Intercom] Error acquiring microphone', err);
      stopPushToTalk();

      const errName = err ? (err.name || '') : '';
      const errMsg = err ? (err.message || String(err)) : '';

      if (
        errName === 'InsecureContextError' ||
        !isSecureOriginForMicrophone() ||
        errMsg.includes('getUserMedia') ||
        errMsg.includes('mediaDevices') ||
        errMsg.includes('undefined')
      ) {
        openMicGuideModal();
      } else if (errName === 'NotAllowedError' || errName === 'PermissionDeniedError') {
        alert('Quyền truy cập micro đã bị từ chối. Vui lòng cho phép quyền micro trong cài đặt trang web của trình duyệt.');
      } else if (errName === 'NotFoundError' || errName === 'DevicesNotFoundError') {
        alert('Không tìm thấy thiết bị thu âm (micro) trên thiết bị của bạn.');
      } else {
        alert('Không thể mở micro máy tính/điện thoại: ' + (errMsg || 'Lỗi không xác định'));
      }
    }
  }

  function stopPushToTalk() {
    if (!isTalking) return;
    isTalking = false;

    if (btnPushToTalk) {
      btnPushToTalk.classList.remove('talking');
    }
    if (pttText) {
      pttText.textContent = 'Giữ để nói (Đàm thoại 2 chiều)';
    }
    if (pttStatus) {
      pttStatus.textContent = 'Nhấn và giữ nút để truyền giọng nói đến máy tính bảng';
      pttStatus.style.color = '';
    }

    try {
      if (micProcessorNode) {
        micProcessorNode.disconnect();
        micProcessorNode = null;
      }
      if (micAudioContext) {
        micAudioContext.close().catch(() => {});
        micAudioContext = null;
      }
      if (micMediaStream) {
        micMediaStream.getTracks().forEach(t => t.stop());
        micMediaStream = null;
      }
    } catch (e) {
      console.warn('[Intercom] Error cleaning up mic', e);
    }
    pttPendingChunks = [];

    if (!isAudioActive && audioWs) {
      try { audioWs.close(); } catch (_) {}
      audioWs = null;
    }
  }

  if (btnToggleAudio) {
    btnToggleAudio.addEventListener('click', toggleLiveAudio);
  }

  if (sliderAudioVolume) {
    sliderAudioVolume.addEventListener('input', (e) => {
      const val = parseInt(e.target.value, 10);
      e.target.title = `Âm lượng: ${val}%`;
      if (audioGainNode) {
        audioGainNode.gain.value = val / 100;
      }
    });
  }

  if (btnPushToTalk) {
    // Prevent context menu disruption during touch-and-hold on mobile
    btnPushToTalk.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      return false;
    });

    btnPushToTalk.addEventListener('mousedown', (e) => {
      e.preventDefault();
      startPushToTalk();
    });
    window.addEventListener('mouseup', () => {
      if (isTalking) stopPushToTalk();
    });
    window.addEventListener('blur', () => {
      if (isTalking) stopPushToTalk();
    });

    btnPushToTalk.addEventListener('touchstart', (e) => {
      e.preventDefault();
      startPushToTalk();
    }, { passive: false });
    btnPushToTalk.addEventListener('touchend', (e) => {
      e.preventDefault();
      stopPushToTalk();
    }, { passive: false });
    btnPushToTalk.addEventListener('touchcancel', (e) => {
      e.preventDefault();
      stopPushToTalk();
    }, { passive: false });
  }

  // Microphone Permission Guidance Modal Listeners
  if (closeMicGuideModalBtn) {
    closeMicGuideModalBtn.addEventListener('click', closeMicGuideModal);
  }
  if (closeMicGuideFooterBtn) {
    closeMicGuideFooterBtn.addEventListener('click', closeMicGuideModal);
  }
  if (micGuideModal) {
    micGuideModal.addEventListener('click', (e) => {
      if (e.target === micGuideModal) {
        closeMicGuideModal();
      }
    });
  }
  if (btnCopyFlagUrl) {
    btnCopyFlagUrl.addEventListener('click', () => {
      const val = browserFlagUrlInput ? browserFlagUrlInput.value : getBrowserFlagUrl();
      copyTextToClipboard(val);
    });
  }
  if (btnCopyOriginUrl) {
    btnCopyOriginUrl.addEventListener('click', () => {
      const val = cameraOriginInput ? cameraOriginInput.value : (window.location.origin || '');
      copyTextToClipboard(val);
    });
  }
  if (browserFlagUrlInput) {
    browserFlagUrlInput.addEventListener('click', () => {
      browserFlagUrlInput.select();
    });
  }
  if (cameraOriginInput) {
    cameraOriginInput.addEventListener('click', () => {
      cameraOriginInput.select();
    });
  }

  // ==========================================================================
  // Fullscreen Management with Cross-Browser Native API + Mobile Pseudo-Fullscreen Fallback
  // ==========================================================================
  let fullscreenControlsTimeout = null;
  const CONTROLS_HIDE_DELAY = 3500;
  let hasPushedFullscreenState = false;
  let lastTouchTime = 0;

  function isNativeFullscreen() {
    return !!(
      document.fullscreenElement ||
      document.webkitFullscreenElement ||
      document.mozFullScreenElement ||
      document.msFullscreenElement
    );
  }

  function isPseudoFullscreen() {
    const container = document.getElementById('videoContainer');
    return container ? container.classList.contains('pseudo-fullscreen') : false;
  }

  function isAnyFullscreen() {
    return isNativeFullscreen() || isPseudoFullscreen();
  }

  function showFullscreenControls() {
    const container = document.getElementById('videoContainer');
    if (!container) return;
    container.classList.remove('controls-hidden');
    resetFullscreenControlsTimer();
  }

  function hideFullscreenControls() {
    const container = document.getElementById('videoContainer');
    if (!container) return;
    if (isAnyFullscreen()) {
      container.classList.add('controls-hidden');
    }
  }

  function resetFullscreenControlsTimer() {
    if (fullscreenControlsTimeout) {
      clearTimeout(fullscreenControlsTimeout);
      fullscreenControlsTimeout = null;
    }
    if (isAnyFullscreen()) {
      fullscreenControlsTimeout = setTimeout(() => {
        hideFullscreenControls();
      }, CONTROLS_HIDE_DELAY);
    }
  }

  function pushFullscreenHistoryState() {
    if (!hasPushedFullscreenState) {
      try {
        history.pushState({ watchcam_fullscreen: true }, '');
        hasPushedFullscreenState = true;
      } catch (_) {}
    }
  }

  function popFullscreenHistoryState() {
    if (hasPushedFullscreenState) {
      hasPushedFullscreenState = false;
      if (window.history && window.history.state && window.history.state.watchcam_fullscreen) {
        try {
          window.history.back();
        } catch (_) {}
      }
    }
  }

  function updateFullscreenUI(active) {
    const container = document.getElementById('videoContainer');
    if (!container) return;

    if (active) {
      container.classList.add('fullscreen-active');
      if (fullscreenBtn) {
        fullscreenBtn.textContent = 'Thu nhỏ';
        fullscreenBtn.title = 'Thu nhỏ (Thoát toàn màn hình)';
      }
      if (exitFullscreenTopBtn) {
        exitFullscreenTopBtn.style.display = 'inline-flex';
      }
      showFullscreenControls();
    } else {
      container.classList.remove('fullscreen-active');
      container.classList.remove('pseudo-fullscreen');
      document.body.classList.remove('pseudo-fullscreen-active');
      document.documentElement.classList.remove('pseudo-fullscreen-active');
      if (fullscreenBtn) {
        fullscreenBtn.textContent = 'Toàn màn hình';
        fullscreenBtn.title = 'Toàn màn hình';
      }
      if (exitFullscreenTopBtn) {
        exitFullscreenTopBtn.style.display = 'none';
      }
      if (fullscreenControlsTimeout) {
        clearTimeout(fullscreenControlsTimeout);
        fullscreenControlsTimeout = null;
      }
      container.classList.remove('controls-hidden');
    }

    setTimeout(() => {
      if (typeof applyRotation === 'function') {
        applyRotation(currentRotation);
      }
    }, 100);
  }

  function enterPseudoFullscreen() {
    const container = document.getElementById('videoContainer');
    if (!container) return;
    container.classList.add('pseudo-fullscreen');
    document.body.classList.add('pseudo-fullscreen-active');
    document.documentElement.classList.add('pseudo-fullscreen-active');
    pushFullscreenHistoryState();
    updateFullscreenUI(true);
  }

  function exitPseudoFullscreen() {
    const container = document.getElementById('videoContainer');
    if (container) {
      container.classList.remove('pseudo-fullscreen');
    }
    document.body.classList.remove('pseudo-fullscreen-active');
    document.documentElement.classList.remove('pseudo-fullscreen-active');
    updateFullscreenUI(false);
  }

  async function enterAnyFullscreen() {
    const container = document.getElementById('videoContainer');
    if (!container) return;

    // Detect vendor-prefixed native Fullscreen API methods
    const reqMethod = container.requestFullscreen ||
                      container.webkitRequestFullscreen ||
                      container.mozRequestFullScreen ||
                      container.msRequestFullscreen;

    const nativeAvailable = typeof reqMethod === 'function' &&
      (document.fullscreenEnabled !== false && document.webkitFullscreenEnabled !== false);

    if (nativeAvailable) {
      try {
        const promise = reqMethod.call(container);
        if (promise && typeof promise.then === 'function') {
          await promise;
        }
        if (isNativeFullscreen()) {
          pushFullscreenHistoryState();
          updateFullscreenUI(true);
          return;
        }
        // In older WebKit / Safari where reqMethod returns undefined synchronously,
        // wait up to 150ms for asynchronous fullscreen transition
        await new Promise(resolve => setTimeout(resolve, 150));
        if (isNativeFullscreen()) {
          pushFullscreenHistoryState();
          updateFullscreenUI(true);
          return;
        }
      } catch (err) {
        console.warn('Native requestFullscreen failed, switching to guaranteed pseudo-fullscreen:', err);
      }
    }

    // Fallback: guaranteed full-viewport CSS pseudo-fullscreen (iOS Safari / restricted mobile webviews)
    enterPseudoFullscreen();
  }

  function exitAnyFullscreen(popHistory = true) {
    if (popHistory) {
      popFullscreenHistoryState();
    }

    if (isPseudoFullscreen()) {
      exitPseudoFullscreen();
    }

    if (isNativeFullscreen()) {
      try {
        const exitMethod = document.exitFullscreen ||
                           document.webkitExitFullscreen ||
                           document.mozCancelFullScreen ||
                           document.msExitFullscreen;
        if (typeof exitMethod === 'function') {
          const res = exitMethod.call(document);
          if (res && typeof res.catch === 'function') {
            res.catch(() => {});
          }
        }
      } catch (err) {
        console.warn('Native exitFullscreen error:', err);
      }
    }

    updateFullscreenUI(false);
  }

  function toggleFullscreen() {
    if (isAnyFullscreen()) {
      exitAnyFullscreen(true);
    } else {
      enterAnyFullscreen();
    }
  }

  if (fullscreenBtn) {
    fullscreenBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      toggleFullscreen();
    });
  }

  if (exitFullscreenTopBtn) {
    exitFullscreenTopBtn.addEventListener('click', (e) => {
      e.stopPropagation();
      exitAnyFullscreen(true);
    });
  }

  // Handle tap / click inside videoContainer to show/hide controls in fullscreen mode
  const videoContainerEl = document.getElementById('videoContainer');
  if (videoContainerEl) {
    // Record touch timestamp to ignore synthetic mousemove events on mobile devices
    videoContainerEl.addEventListener('touchstart', () => {
      lastTouchTime = Date.now();
      if (isAnyFullscreen()) {
        resetFullscreenControlsTimer();
      }
    }, { passive: true });

    const handlePointerMove = (e) => {
      if (!isAnyFullscreen()) return;
      // If event is from a touchscreen or occurred shortly after a touch tap, ignore synthetic mousemove
      if (e.pointerType && e.pointerType !== 'mouse') return;
      if (Date.now() - lastTouchTime < 1000) return;
      showFullscreenControls();
    };

    if (window.PointerEvent) {
      videoContainerEl.addEventListener('pointermove', handlePointerMove);
    } else {
      videoContainerEl.addEventListener('mousemove', handlePointerMove);
    }

    videoContainerEl.addEventListener('click', (e) => {
      if (!isAnyFullscreen()) return;

      // If user tapped a button, input, or inside overlay bars, do not toggle visibility
      if (e.target.closest('button, a, input, select, label, .stream-overlay-top, .stream-overlay-bottom')) {
        resetFullscreenControlsTimer();
        return;
      }

      if (videoContainerEl.classList.contains('controls-hidden')) {
        showFullscreenControls();
      } else {
        hideFullscreenControls();
      }
    });
  }

  // Escape key handler
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' || e.key === 'Esc') {
      if (micGuideModal && micGuideModal.style.display !== 'none') {
        closeMicGuideModal();
      }
      if (isAnyFullscreen()) {
        exitAnyFullscreen(true);
      }
    }
  });

  // Browser back button / Android gesture back handler
  window.addEventListener('popstate', () => {
    if (hasPushedFullscreenState || isAnyFullscreen()) {
      hasPushedFullscreenState = false;
      exitAnyFullscreen(false);
    }
  });

  // Native Fullscreen change events
  ['fullscreenchange', 'webkitfullscreenchange', 'mozfullscreenchange', 'MSFullscreenChange'].forEach(evt => {
    document.addEventListener(evt, () => {
      const isFull = isNativeFullscreen();
      if (isFull) {
        const container = document.getElementById('videoContainer');
        if (container && container.classList.contains('pseudo-fullscreen')) {
          container.classList.remove('pseudo-fullscreen');
          document.body.classList.remove('pseudo-fullscreen-active');
          document.documentElement.classList.remove('pseudo-fullscreen-active');
        }
        pushFullscreenHistoryState();
        updateFullscreenUI(true);
      } else if (!isPseudoFullscreen()) {
        hasPushedFullscreenState = false;
        updateFullscreenUI(false);
      }
    });
  });

  // Native Fullscreen error events -> fallback to pseudo-fullscreen
  ['fullscreenerror', 'webkitfullscreenerror', 'mozfullscreenerror', 'MSFullscreenError'].forEach(evt => {
    document.addEventListener(evt, () => {
      console.warn('Native fullscreen error event triggered, activating pseudo-fullscreen fallback');
      if (!isNativeFullscreen() && !isPseudoFullscreen()) {
        enterPseudoFullscreen();
      }
    });
  });

  quickSnapshotBtn.addEventListener('click', async () => {
    try {
      const tokenQuery = currentToken ? `?token=${encodeURIComponent(currentToken)}` : '';
      const res = await apiFetch(`/api/snapshot${tokenQuery}`);
      if (!res.ok) throw new Error('Không lấy được ảnh');
      const blob = await res.blob();
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `watchcam_${Date.now()}.jpg`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      alert('Không thể chụp ảnh: ' + e.message);
    }
  });

  // Pairing Modal
  function showPairModal(show) {
    const isCurrentlyShown = pairModal.style.display === 'flex';
    pairModal.style.display = show ? 'flex' : 'none';
    authBtn.style.display = show ? 'none' : 'inline-block';
    logoutBtn.style.display = show ? 'none' : 'inline-block';
    if (show) {
      if (!isCurrentlyShown) {
        pairCodeInput.value = '';
        pairError.style.display = 'none';
        pairCodeInput.focus();
      }
    }
  }

  submitPairBtn.addEventListener('click', async () => {
    const code = pairCodeInput.value.trim().replace(/\s+/g, '');
    if (!code) {
      pairError.textContent = 'Vui lòng nhập mã ghép nối!';
      pairError.style.display = 'block';
      return;
    }
    submitPairBtn.disabled = true;
    submitPairBtn.textContent = 'Đang ghép nối...';
    try {
      const res = await fetch('/api/auth/pair', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ code: code })
      });
      const data = await res.json();
      if (res.ok && data.success && data.token) {
        saveDeviceToken(data.token);
        showPairModal(false);
        initStream();
        connectWebSocket();
        fetchStatus();
        fetchEvents();
      } else {
        pairError.textContent = data.message || 'Mã ghép nối không đúng hoặc đã hết hạn!';
        pairError.style.display = 'block';
      }
    } catch (err) {
      pairError.textContent = 'Lỗi kết nối máy chủ: ' + err.message;
      pairError.style.display = 'block';
    } finally {
      submitPairBtn.disabled = false;
      submitPairBtn.textContent = 'Xác Nhận Ghép Nối';
    }
  });

  pairCodeInput.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      submitPairBtn.click();
    }
  });

  authBtn.addEventListener('click', () => showPairModal(true));

  logoutBtn.addEventListener('click', async () => {
    if (confirm('Bạn có chắc chắn muốn đăng xuất phiên xem này?')) {
      try {
        await apiFetch('/api/auth/logout', { method: 'POST' });
      } catch (_) {}
      clearDeviceToken();
      clearTimeout(wsReconnectTimer);
      if (ws) {
        const oldWs = ws;
        ws = null;
        oldWs.close();
      }
      streamView.src = '';
      showPairModal(true);
    }
  });

  // WebSocket Connection
  function connectWebSocket() {
    if (!currentToken) return;
    if (ws) {
      const oldWs = ws;
      ws = null;
      oldWs.close();
    }
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const tokenQuery = currentToken ? `?token=${encodeURIComponent(currentToken)}` : '';
    const wsUrl = `${protocol}//${window.location.host}/ws/events${tokenQuery}`;

    ws = new WebSocket(wsUrl);

    ws.onopen = () => {
      wsRetryCount = 0;
      setConnectionStatus(true);
    };

    ws.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data);
        handleWsMessage(msg);
      } catch (e) {
        console.error('Lỗi phân tích WebSocket payload:', e);
      }
    };

    ws.onclose = () => {
      setConnectionStatus(false);
      if (!currentToken) {
        return;
      }
      // Exponential backoff reconnect
      const delay = Math.min(1000 * Math.pow(1.5, wsRetryCount), 15000);
      wsRetryCount++;
      clearTimeout(wsReconnectTimer);
      wsReconnectTimer = setTimeout(connectWebSocket, delay);
    };

    ws.onerror = (err) => {
      console.warn('WebSocket error:', err);
      if (ws) {
        ws.close();
      }
    };
  }

  function setConnectionStatus(online) {
    if (online) {
      connBadge.className = 'status-badge online';
      connStatusText.textContent = 'Đang kết nối';
    } else {
      connBadge.className = 'status-badge offline';
      connStatusText.textContent = 'Mất kết nối';
    }
  }

  function handleWsMessage(msg) {
    if (msg.type === 'motion_detected' || msg.type === 'person_detected' || msg.type === 'object_detected') {
      const scorePct = Math.round((msg.motionScore || 0) * 100);
      const confPct = Math.round(((msg.confidence !== undefined ? msg.confidence : msg.motionScore) || 0) * 100);
      const entity = msg.entityType || (msg.type === 'person_detected' ? 'person' : msg.type === 'object_detected' ? 'object' : 'motion');

      motionScoreVal.textContent = `${scorePct}%`;
      motionScoreBar.style.width = `${Math.min(scorePct * 3, 100)}%`;

      // Highlight alert banner with entity classification
      let bannerClass = 'alert-banner active';
      let titleText = `Phát hiện chuyển động (${scorePct}%)`;
      if (entity === 'person') {
        bannerClass = 'alert-banner active alert-person';
        titleText = `Phát hiện người (${confPct}%)`;
      } else if (entity === 'object') {
        bannerClass = 'alert-banner active alert-object';
        titleText = `Phát hiện vật thể (${confPct}%)`;
      }

      latestAlertBanner.className = bannerClass;
      alertTitle.textContent = titleText;
      alertTime.textContent = new Date(msg.timestamp).toLocaleTimeString('vi-VN');

      const bbox = msg.boundingBox || msg.region;
      if (bbox) {
        renderDetectionBox(bbox, entity, msg.confidence);
      }

      if (alertQuickViewBtn) {
        alertQuickViewBtn.style.display = 'inline-block';
        alertQuickViewBtn.onclick = () => {
          openSnapshotModal(msg.eventId);
        };
      }

      playAlertBeep();

      // Refresh event list to show snapshot
      setTimeout(fetchEvents, 300);

      setTimeout(() => {
        latestAlertBanner.className = 'alert-banner idle';
        alertTitle.textContent = 'Không gian yên tĩnh';
        alertTime.textContent = 'Đang tiếp tục giám sát...';
        if (alertQuickViewBtn) {
          alertQuickViewBtn.style.display = 'none';
        }
        motionScoreBar.style.width = '0%';
        motionScoreVal.textContent = '0%';
      }, 4500);
    } else if (msg.type === 'detection_update' || msg.type === 'live_detection') {
      const scorePct = Math.round((msg.motionScore || 0) * 100);
      motionScoreVal.textContent = `${scorePct}%`;
      motionScoreBar.style.width = `${Math.min(scorePct * 3, 100)}%`;
      const entity = msg.entityType || 'motion';
      const bbox = msg.boundingBox || msg.region;
      if (bbox) {
        renderDetectionBox(bbox, entity, msg.confidence);
      } else {
        renderDetectionBox(null);
      }
    } else if (msg.type === 'video_recorded') {
      const ev = currentEvents.find(e => e.id === msg.eventId);
      if (ev) {
        ev.hasVideo = true;
        renderEvents(currentEvents);
      } else {
        fetchEvents();
      }

      // If modal is currently open viewing this event, automatically switch to video and play!
      if (selectedEvent && selectedEvent.id === msg.eventId && snapshotModal.style.display === 'flex') {
        selectedEvent.hasVideo = true;
        const tokenQuery = currentToken ? `?token=${encodeURIComponent(currentToken)}` : '';
        const videoUrl = `/api/events/${selectedEvent.id}/video${tokenQuery}`;
        modalSnapshotImg.style.display = 'none';
        modalVideoPlayer.style.display = 'block';
        modalVideoPlayer.src = videoUrl;
        modalVideoPlayer.muted = false;
        modalVideoPlayer.volume = 1.0;
        modalVideoPlayer.load();
        modalVideoPlayer.play().catch(() => {
          modalVideoPlayer.muted = true;
          modalVideoPlayer.play().catch(() => {});
        });

        downloadVideoBtn.href = videoUrl;
        downloadVideoBtn.download = `motion_${selectedEvent.id}.mp4`;
        downloadVideoBtn.style.display = 'inline-block';

        toggleMediaViewBtn.style.display = 'inline-block';
        toggleMediaViewBtn.textContent = 'Xem Ảnh Chụp';
      }
    } else if (msg.type === 'status_update') {
      updateStatusUI(msg.status);
    } else if (msg.type === 'config_updated') {
      updateStatusUI(msg);
    } else if (msg.type === 'device_status') {
      updateDeviceHealthUI(msg.device);
    } else if (msg.type === 'sessions_revoked' || msg.type === 'auth_revoked') {
      clearDeviceToken();
      clearTimeout(wsReconnectTimer);
      if (ws) {
        const oldWs = ws;
        ws = null;
        oldWs.close();
      }
      streamView.src = '';
      showPairModal(true);
    }
  }

  // Polling Status
  async function fetchStatus() {
    try {
      const tokenQuery = currentToken ? `?token=${encodeURIComponent(currentToken)}` : '';
      const res = await apiFetch(`/api/status${tokenQuery}`);
      if (!res.ok) return;
      const status = await res.json();
      if (status.isAuthenticated === false) {
        if (currentToken) {
          clearDeviceToken();
          clearTimeout(wsReconnectTimer);
          if (ws) {
            const oldWs = ws;
            ws = null;
            oldWs.close();
          }
          streamView.src = '';
          showPairModal(true);
        }
      } else {
        if (currentToken) {
          showPairModal(false);
        }
        updateStatusUI(status);
      }
    } catch (e) {
      console.warn('Lỗi lấy trạng thái:', e);
    }
  }

  function updateMotionCardUI(enabled) {
    if (!enabled) {
      motionScoreVal.textContent = '0% (Đã tắt)';
      motionScoreBar.style.width = '0%';
      latestAlertBanner.className = 'alert-banner idle';
      alertTitle.textContent = 'Đã tắt giám sát chuyển động';
      alertTime.textContent = 'Bật công tắc trên để bắt đầu phát hiện';
    }
  }

  function updateStatusUI(status) {
    if (!status) return;
    if (status.httpsPort) {
      currentHttpsPort = status.httpsPort;
    }
    viewerCount.textContent = `${status.viewerCount || 0} người xem`;
    streamFpsTag.textContent = `${(status.actualFps || 0).toFixed(1)} FPS`;
    cameraNameTag.textContent = status.cameraName || 'Camera Chính';
    if (status.audioLevel !== undefined && !isAudioActive) {
      updateAudioVuMeter(status.audioLevel);
    }
    if (status.motionEnabled !== undefined && !motionToggle.matches(':active')) {
      motionToggle.checked = status.motionEnabled;
      updateMotionCardUI(status.motionEnabled);
    }
    if (status.sensitivity !== undefined && !sensitivityInput.matches(':active')) {
      sensitivityInput.value = status.sensitivity;
      sensitivityVal.textContent = status.sensitivity;
    }
    if (status.streamQuality !== undefined && streamQualitySelect && !streamQualitySelect.matches(':focus')) {
      streamQualitySelect.value = status.streamQuality.toString();
    }
    if (status.resolution !== undefined && streamResolutionSelect && !streamResolutionSelect.matches(':focus')) {
      streamResolutionSelect.value = status.resolution;
    }
    if (status.cooldownSeconds !== undefined && cooldownSelect && !cooldownSelect.matches(':focus')) {
      cooldownSelect.value = status.cooldownSeconds.toString();
    }
    if (status.emailAlertsEnabled !== undefined && emailAlertsToggle && !emailAlertsToggle.matches(':active')) {
      emailAlertsToggle.checked = status.emailAlertsEnabled;
    }
    if (status.smtpHost && smtpHostInput && !smtpHostInput.matches(':focus')) {
      smtpHostInput.value = status.smtpHost;
    }
    if (status.smtpPort && smtpPortInput && !smtpPortInput.matches(':focus')) {
      smtpPortInput.value = status.smtpPort.toString();
    }
    if (status.smtpUsername !== undefined && smtpUsernameInput && !smtpUsernameInput.matches(':focus')) {
      smtpUsernameInput.value = status.smtpUsername;
    }
    if (status.smtpRecipient !== undefined && smtpRecipientInput && !smtpRecipientInput.matches(':focus')) {
      smtpRecipientInput.value = status.smtpRecipient;
    }
    if (status.device) {
      updateDeviceHealthUI(status.device);
    }
    if (status.isMonitoring) {
      startCamBtn.style.opacity = '0.5';
      stopCamBtn.style.opacity = '1.0';
    } else {
      startCamBtn.style.opacity = '1.0';
      stopCamBtn.style.opacity = '0.5';
    }
  }

  // Camera Controls
  startCamBtn.addEventListener('click', async () => {
    try {
      await apiFetch('/api/camera/start', { method: 'POST' });
      initStream();
      fetchStatus();
    } catch (e) {
      alert('Lỗi bắt đầu camera: ' + e.message);
    }
  });

  stopCamBtn.addEventListener('click', async () => {
    try {
      await apiFetch('/api/camera/stop', { method: 'POST' });
      fetchStatus();
    } catch (e) {
      alert('Lỗi dừng camera: ' + e.message);
    }
  });

  switchCamBtn.addEventListener('click', async () => {
    try {
      await apiFetch('/api/config', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ switchCamera: true })
      });
      fetchStatus();
    } catch (e) {
      alert('Lỗi chuyển camera: ' + e.message);
    }
  });

  streamQualitySelect.addEventListener('change', async () => {
    const q = parseInt(streamQualitySelect.value, 10);
    try {
      await apiFetch('/api/config', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ streamQuality: q })
      });
    } catch (e) {
      console.warn('Lỗi cập nhật chất lượng:', e);
    }
  });

  if (streamResolutionSelect) {
    streamResolutionSelect.addEventListener('change', async () => {
      const resVal = streamResolutionSelect.value;
      try {
        await apiFetch('/api/config', {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ resolution: resVal })
        });
      } catch (e) {
        console.warn('Lỗi cập nhật độ phân giải:', e);
      }
    });
  }

  if (cooldownSelect) {
    cooldownSelect.addEventListener('change', async () => {
      const cd = parseInt(cooldownSelect.value, 10);
      try {
        await apiFetch('/api/config', {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ cooldownSeconds: cd })
        });
      } catch (e) {
        console.warn('Lỗi cập nhật cooldown:', e);
      }
    });
  }

  motionToggle.addEventListener('change', async () => {
    const isChecked = motionToggle.checked;
    updateMotionCardUI(isChecked);
    try {
      const res = await apiFetch('/api/config', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ motionEnabled: isChecked })
      });
      if (!res.ok) {
        throw new Error('HTTP ' + res.status);
      }
    } catch (e) {
      console.warn('Lỗi bật/tắt phát hiện:', e);
      motionToggle.checked = !isChecked;
      updateMotionCardUI(!isChecked);
    }
  });

  sensitivityInput.addEventListener('input', () => {
    sensitivityVal.textContent = sensitivityInput.value;
  });

  sensitivityInput.addEventListener('change', async () => {
    const val = parseInt(sensitivityInput.value, 10);
    try {
      await apiFetch('/api/config', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ sensitivity: val })
      });
    } catch (e) {
      console.warn('Lỗi đổi độ nhạy:', e);
    }
  });

  // Email Alert Handlers
  function showEmailStatus(msg, type = 'info') {
    if (!emailStatusMsg) return;
    emailStatusMsg.textContent = msg;
    emailStatusMsg.className = `status-msg ${type}`;
    emailStatusMsg.style.display = 'block';
    if (type !== 'error') {
      setTimeout(() => { emailStatusMsg.style.display = 'none'; }, 4000);
    }
  }

  if (emailAlertsToggle) {
    emailAlertsToggle.addEventListener('change', async () => {
      const isChecked = emailAlertsToggle.checked;
      try {
        await apiFetch('/api/config', {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            emailAlertsEnabled: isChecked
          })
        });
        showEmailStatus(isChecked ? 'Đã bật thông báo qua Gmail!' : 'Đã tắt thông báo qua Gmail', 'info');
      } catch (e) {
        console.warn('Lỗi bật/tắt email:', e);
        emailAlertsToggle.checked = !isChecked;
      }
    });
  }

  if (saveEmailConfigBtn) {
    saveEmailConfigBtn.addEventListener('click', async () => {
      const patchData = {
        emailAlertsEnabled: emailAlertsToggle ? emailAlertsToggle.checked : true,
        smtpHost: smtpHostInput ? smtpHostInput.value.trim() : 'smtp.gmail.com',
        smtpPort: smtpPortInput ? parseInt(smtpPortInput.value, 10) : 587,
        smtpUsername: smtpUsernameInput ? smtpUsernameInput.value.trim() : '',
        smtpRecipient: smtpRecipientInput ? smtpRecipientInput.value.trim() : ''
      };
      if (smtpPasswordInput && smtpPasswordInput.value) {
        patchData.smtpPassword = smtpPasswordInput.value.trim();
      }

      try {
        showEmailStatus('Đang lưu cài đặt email...', 'info');
        const res = await apiFetch('/api/config', {
          method: 'PATCH',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(patchData)
        });
        if (res.ok) {
          showEmailStatus('Đã lưu cấu hình Gmail SMTP thành công!', 'success');
        } else {
          showEmailStatus('Lỗi khi lưu cấu hình: HTTP ' + res.status, 'error');
        }
      } catch (e) {
        showEmailStatus('Lỗi kết nối: ' + e.message, 'error');
      }
    });
  }

  if (testEmailBtn) {
    testEmailBtn.addEventListener('click', async () => {
      try {
        showEmailStatus('Đang yêu cầu camera gửi email thử nghiệm...', 'info');
        const res = await apiFetch('/api/email/test', { method: 'POST' });
        if (res.ok) {
          showEmailStatus('Đã kích hoạt gửi email thử nghiệm! Vui lòng kiểm tra hộp thư Gmail.', 'success');
        } else {
          showEmailStatus('Không thể kích hoạt gửi email: HTTP ' + res.status, 'error');
        }
      } catch (e) {
        showEmailStatus('Lỗi: ' + e.message, 'error');
      }
    });
  }

  // Events History
  async function fetchEvents() {
    try {
      const res = await apiFetch('/api/events');
      if (!res.ok) return;
      currentEvents = await res.json();
      renderEvents(currentEvents);
    } catch (e) {
      console.warn('Lỗi lấy sự kiện:', e);
    }
  }

  function renderEvents(events) {
    eventCountHeader.textContent = events ? events.length : 0;
    if (!events || events.length === 0) {
      eventList.innerHTML = '<div class="empty-state">Chưa có sự kiện chuyển động nào</div>';
      return;
    }
    const now = Date.now();
    eventList.innerHTML = events.map(ev => {
      const timeStr = new Date(ev.timestamp).toLocaleString('vi-VN');
      const scorePct = Math.round((ev.motionScore || ev.motion_score || 0) * 100);
      const confPct = Math.round(((ev.confidence !== undefined ? ev.confidence : (ev.motionScore || ev.motion_score)) || 0) * 100);
      const entity = ev.entityType || ev.entity_type || (ev.eventType === 'person_detected' || ev.event_type === 'person_detected' ? 'person' : ev.eventType === 'object_detected' || ev.event_type === 'object_detected' ? 'object' : 'motion');
      const tokenQuery = currentToken ? `?token=${encodeURIComponent(currentToken)}` : '';
      const snapshotUrl = `/api/events/${ev.id}/snapshot${tokenQuery}`;
      const isRecent = (now - ev.timestamp) < 6000;

      let entityBadge = '';
      if (entity === 'person') {
        entityBadge = `<span class="badge-entity badge-person">Người ${confPct}%</span>`;
      } else if (entity === 'object') {
        entityBadge = `<span class="badge-entity badge-object">Vật thể ${confPct}%</span>`;
      } else {
        entityBadge = `<span class="badge-entity badge-motion">Chuyển động</span>`;
      }

      let badgeHtml = '';
      if (ev.hasVideo) {
        badgeHtml = '<span class="badge-video">Video Clip</span>';
      } else if (isRecent) {
        badgeHtml = '<span class="badge-recording">Đang quay clip...</span>';
      }
      return `
        <div class="event-item" data-id="${ev.id}">
          <img src="${snapshotUrl}" class="event-thumb" alt="Ảnh thu nhỏ" onerror="this.src='data:image/svg+xml;utf8,<svg xmlns=\\'http://www.w3.org/2000/svg\\' width=\\'56\\' height=\\'42\\' fill=\\'%2330363d\\'><rect width=\\'100%\\' height=\\'100%\\'/></svg>'" />
          <div class="event-info">
            <div class="event-time">${timeStr} ${badgeHtml} ${entityBadge}</div>
            <div class="event-sub">Độ biến đổi: ${scorePct}% &bull; ${ev.camera || 'Camera'}</div>
          </div>
        </div>
      `;
    }).join('');

    eventList.querySelectorAll('.event-item').forEach(el => {
      el.addEventListener('click', () => {
        const id = el.getAttribute('data-id');
        openSnapshotModal(id);
      });
    });
  }

  async function openSnapshotModal(id) {
    selectedEvent = currentEvents.find(e => e.id === id);
    if (!selectedEvent) {
      try {
        const tokenQuery = currentToken ? `?token=${encodeURIComponent(currentToken)}` : '';
        const res = await apiFetch(`/api/events/${id}${tokenQuery}`);
        if (res.ok) {
          selectedEvent = await res.json();
        }
      } catch (_) {}
    }
    if (!selectedEvent) {
      selectedEvent = { id: id, timestamp: Date.now(), motionScore: 0.8, camera: 'Camera', hasVideo: false };
    }
    const timeStr = new Date(selectedEvent.timestamp).toLocaleString('vi-VN');
    const scorePct = Math.round((selectedEvent.motionScore || selectedEvent.motion_score || 0) * 100);
    const confPct = Math.round(((selectedEvent.confidence !== undefined ? selectedEvent.confidence : (selectedEvent.motionScore || selectedEvent.motion_score)) || 0) * 100);
    const entity = selectedEvent.entityType || selectedEvent.entity_type || (selectedEvent.eventType === 'person_detected' || selectedEvent.event_type === 'person_detected' ? 'person' : selectedEvent.eventType === 'object_detected' || selectedEvent.event_type === 'object_detected' ? 'object' : 'motion');

    const tokenQuery = currentToken ? `?token=${encodeURIComponent(currentToken)}` : '';
    const snapshotUrl = `/api/events/${selectedEvent.id}/snapshot${tokenQuery}`;
    const videoUrl = `/api/events/${selectedEvent.id}/video${tokenQuery}`;

    if (modalEventTitle) {
      const typeTitle = entity === 'person' ? 'Phát Hiện Người' : (entity === 'object' ? 'Phát Hiện Vật Thể' : 'Phát Hiện Chuyển Động');
      modalEventTitle.textContent = `${typeTitle} (${confPct}%) - ${selectedEvent.id}`;
    }
    modalEventTime.textContent = timeStr;
    modalEventScore.textContent = `Mức chuyển động: ${scorePct}%`;
    modalEventCamera.textContent = selectedEvent.camera || 'Camera';
    downloadSnapshotBtn.href = snapshotUrl;
    downloadSnapshotBtn.download = `snapshot_${selectedEvent.id}.jpg`;
    modalSnapshotImg.src = snapshotUrl;

    const bbox = selectedEvent.boundingBox || selectedEvent.region;
    if (modalDetectionBox) {
      if (bbox && bbox.length >= 4 && (bbox[2] > 0 || bbox[3] > 0)) {
        const [bx, by, bw, bh] = bbox;
        modalDetectionBox.className = `modal-detection-box ${entity}`;
        modalDetectionBox.style.left = `${(bx * 100).toFixed(2)}%`;
        modalDetectionBox.style.top = `${(by * 100).toFixed(2)}%`;
        modalDetectionBox.style.width = `${(bw * 100).toFixed(2)}%`;
        modalDetectionBox.style.height = `${(bh * 100).toFixed(2)}%`;
        const tagText = entity === 'person' ? `Người (${confPct}%)` : (entity === 'object' ? `Vật thể (${confPct}%)` : `Chuyển động`);
        modalDetectionBox.innerHTML = `<span class="detection-tag">${tagText}</span>`;
        modalDetectionBox.style.display = 'block';
      } else {
        modalDetectionBox.style.display = 'none';
        modalDetectionBox.innerHTML = '';
      }
    }

    let hasVideo = !!selectedEvent.hasVideo;
    if (!hasVideo) {
      try {
        const checkRes = await fetch(videoUrl, { method: 'HEAD' });
        if (checkRes.ok) {
          hasVideo = true;
          selectedEvent.hasVideo = true;
        }
      } catch (_) {}
    }

    if (hasVideo) {
      // Default to video view and autoplay!
      modalSnapshotImg.style.display = 'none';
      if (modalDetectionBox) modalDetectionBox.style.display = 'none';
      modalVideoPlayer.style.display = 'block';
      modalVideoPlayer.src = videoUrl;
      modalVideoPlayer.muted = false;
      modalVideoPlayer.volume = 1.0;
      modalVideoPlayer.load();
      modalVideoPlayer.play().catch(() => {
        modalVideoPlayer.muted = true;
        modalVideoPlayer.play().catch(() => {});
      });

      downloadVideoBtn.href = videoUrl;
      downloadVideoBtn.download = `motion_${selectedEvent.id}.mp4`;
      downloadVideoBtn.style.display = 'inline-block';

      toggleMediaViewBtn.style.display = 'inline-block';
      toggleMediaViewBtn.textContent = 'Xem Ảnh Chụp';
      toggleMediaViewBtn.onclick = () => {
        if (modalVideoPlayer.style.display === 'none') {
          modalSnapshotImg.style.display = 'none';
          if (modalDetectionBox) modalDetectionBox.style.display = 'none';
          modalVideoPlayer.style.display = 'block';
          modalVideoPlayer.src = videoUrl;
          modalVideoPlayer.muted = false;
          modalVideoPlayer.volume = 1.0;
          modalVideoPlayer.play().catch(() => {
            modalVideoPlayer.muted = true;
            modalVideoPlayer.play().catch(() => {});
          });
          toggleMediaViewBtn.textContent = 'Xem Ảnh Chụp';
        } else {
          modalVideoPlayer.style.display = 'none';
          modalVideoPlayer.pause();
          modalSnapshotImg.style.display = 'block';
          if (modalDetectionBox && bbox && bbox.length >= 4 && (bbox[2] > 0 || bbox[3] > 0)) {
            modalDetectionBox.style.display = 'block';
          }
          toggleMediaViewBtn.textContent = 'Xem Video Clip';
        }
      };
    } else {
      modalVideoPlayer.style.display = 'none';
      modalVideoPlayer.pause();
      modalVideoPlayer.src = '';
      modalSnapshotImg.style.display = 'block';
      downloadVideoBtn.style.display = 'none';
      toggleMediaViewBtn.style.display = 'none';
    }

    snapshotModal.style.display = 'flex';
  }

  closeSnapshotModalBtn.addEventListener('click', () => {
    if (modalVideoPlayer) {
      modalVideoPlayer.pause();
      modalVideoPlayer.src = '';
    }
    if (modalDetectionBox) {
      modalDetectionBox.style.display = 'none';
      modalDetectionBox.innerHTML = '';
    }
    snapshotModal.style.display = 'none';
  });

  deleteEventBtn.addEventListener('click', async () => {
    if (!selectedEvent) return;
    if (confirm('Bạn có muốn xóa sự kiện này không?')) {
      try {
        await apiFetch(`/api/events/${selectedEvent.id}`, { method: 'DELETE' });
        snapshotModal.style.display = 'none';
        fetchEvents();
      } catch (e) {
        alert('Lỗi xóa sự kiện: ' + e.message);
      }
    }
  });

  refreshEventsBtn.addEventListener('click', fetchEvents);

  clearAllEventsBtn.addEventListener('click', async () => {
    if (confirm('Bạn có chắc muốn xóa TẤT CẢ sự kiện và hình ảnh đã lưu?')) {
      try {
        await apiFetch('/api/events/clear', { method: 'POST' });
        fetchEvents();
      } catch (e) {
        alert('Lỗi xóa lịch sử: ' + e.message);
      }
    }
  });

  // Initialization
  applyRotation(currentRotation);
  if (currentToken) {
    // Remembered device: immediately connect and bypass modal completely
    showPairModal(false);
    initStream();
    connectWebSocket();
    fetchStatus();
    fetchEvents();
  } else {
    // New device: prompt for pairing code
    showPairModal(true);
  }
  statusPollTimer = setInterval(fetchStatus, 3000);
})();
