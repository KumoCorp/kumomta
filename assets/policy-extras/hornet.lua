--[[ hornet.lua ]]--
-- Interface functions for Hornet Email Protection integration

local name = "hornet"
local mod = {}
local kumo = require 'kumo'
local utils = require 'policy-extras.policy_utils'


---------------------------------------------------------------------------------------------
--[[ hornet:connect ]]--
-- inputs: 
--   vhost - IP or hostname of the hornet server
--   vparams - array of values incluing port, tls flag, maximum zise to scan, timeout.
--            IE: {['port']=800,['timeout']=30,['use_tls']=true,['maxsize']=2097152}
-- returns: array of hornet server connection values, AKA "hornet connection array"
--
function mod:connect(vhost,vparams)
  return {
    host = vhost or '127.0.0.1',
    port = vparams.port or 8080,
    timeout = vparams.timeout or 10,
    use_tls = vparams.use_tls or false,
  }
end

local function make_url(params, endpoint)
  local vhost = string.format("%s:%d", params.host, params.port)
  local scheme = 'http'
  if params.use_tls then
    scheme = 'https'
  end
		
  return string.format('%s://%s/%s', scheme, vhost, endpoint)
end




---------------------------------------------------------------------------------------------
--[[ hornet:scan ]]--
-- inputs: 
--   params" = hornet connection array, 
--   extraparams = array of:
--     addheaders = true/false   -- Add result headers directly to the message?
--     mode = string            -- make this smtpout (default) to indicate outbound scanning, smtpin for inbound
--     x_sanitize = true/false  -- make this true to have the engine remove dangerous attachments
--   msg = message object
-- returns:
--   state and score (int) - The spam state and spam score returned for the message. For some states, the
--      score may provide additional information about the category the message belongs to.
--      For a complete list of states and scores, please refer to Evaluating State, Score and Verdict.

--   verdict (str) - The message verdict (with no further detail), allowing you to trigger message
--      rejection without further investigating the state and score if not needed. For a
--      complete list of verdicts, please refer to Evaluating State, Score and Verdict.

--   spamcause (str) - The full encoded message spam cause.

--   "-" (str) - The encoded envelope information (if provided) that were passed to the library
--      for scanning (e.g. Inet, Helo, etc.).

--   sanitizeStatus - Status of the sanitize operation:
--[[  • 0: Successfully removed attachments from the original email.
      • -1: Error parsing the message: we could not remove attachments.
      • -2: Malicious attachments may still be present in the modified message.
      ]]--     
--  sanitizedEmail  In case attachments were removed from the original message, the sanitizedEmail
--      field contains the base64 encoded payload of the modified message.

--  internal (str) - A JSON array containing extra data returned by the filter, including the split
--[[      recipients and other metadata depending on the options passed to the filter through
      the AddParams configuration entry.
      The -2 code may be returned in case the message contained a malicious
      attachment which could be removed because of the list
      of SanitizeKeepMime content-types to preserve in the REST
      service configuration; e.g. an EICAR string contained in a text/html part.
      It may contain the matched phishing URL or domain (if any), within
      the InsightUrl=http://www.thisisaphishingurl.com"
      or InsightDomain=thisisaphishingurl.com".
      ]]--

--  Events (array) - The list of current events that matched the message. Typically ["COVID-19", "EMOTET", …].

--  elapsed (str) - Time it took for the server (in ms) to process the message and return the verdict


function mod:scan(params,extraparams,msg)

  if not params then
    print ("No connection table provided")
    return false
  end

 if not msg then
    return false
  end

  -- Flatten message data to a string
  local msgdata = msg:get_data()

  -- decompose msg here into vars to pass
  local modifymsg = extraparams.addheaders or false  -- Add result headers directly to the message?
  local x_inet = msg:get_meta('received_from')       -- IP message was received from
  local x_helo =  msg:get_meta('ehlo_domain')        -- EHLO domain of injected message
  local x_mailfrom = msg:sender().email              -- Envelope Mail FROM
  local x_rcptto = msg:recipient().email             -- Envelope RCPT TO
  local duration = params.timeout or "10s"           -- API call timeout
  local direction_mode = tostring(extraparams.mode) or  'smtpout'  -- make this smtpout to indicate outbound scanning
  local x_sanitize = tostring(extraparams.X_Sanitize) or "false"   -- make this true to have the engine remove dangerous attachments

  --[[ These are ignored in this version but can be set in the Hornet config file ]]--
  --[[
  local get_zippasswd = tostring(extraparams.get_ZipPasswd) or "false"
  local forcewhitelist = tostring(extraparams.ForceWhitelist) or "false"
  local get_events = tostring(get_Events) or "false"
  local get_depthlimit =tostring( get_DepthLimit) or "0"
  ]]--

  local hornetheaders =  {
	['X-Inet'] = x_inet,
        ['X-Helo'] = x_helo,
        ['X-Mailfrom'] = x_mailfrom,
        ['X-Rcptto'] = x_rcptto,
        ['X-Sanitize'] =  x_sanitize,
	['X-Params'] = "mode=" .. direction_mode,
  }

-- catch usecase if a raw number is provided for API timeout
if type(duration) == "number" then
  duration = "" .. tostring(duration) .. "s"
end


  local url = make_url(params, 'api/v1/scan')
  local client = kumo.http.build_client {}
  local response = client
        :post(url)
        :header('Content-Type', 'application/json')
	:headers(hornetheaders)
	:timeout(duration)
        :body(msgdata)
        :send()

      local disposition = string.format(
        '%d %s: %s',
        response:status_code(),
        response:status_reason(),
        response:text()
      )


  if string.sub(disposition,1,8) == "200 OK: " then
	  scanresult = disposition
          scanresult = string.sub(scanresult,9,string.len(scanresult))
  else
	  scanresult = disposition
          print "Security scan failed"
  end

  -- If requested, add headers to message to describe the scan results
  if modifymsg == true then
    scanresult_json = kumo.serde.json_parse(scanresult)
    for skey,svalue in pairs(scanresult_json) do
      skey = "X-Hornet-" .. skey
      msg:append_header(skey, svalue)
    end
  end

  return scanresult
end



return mod
