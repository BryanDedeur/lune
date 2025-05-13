local fs = require("@lune/fs")
local roblox = require("@lune/roblox")
local process = require("@lune/process")

-- Check if input file was provided
local args = process.args
if #args < 1 then
    print("Usage: lune test-place-to-markdown.luau <path-to-place-file> [output-path]")
    process.exit(1)
end

local placePath = args[1]
local outputPath = args[2] or "place-structure.md"

print("Reading place file from: " .. placePath)

-- Read the place file
local placeContents = fs.readFile(placePath)

-- Convert to markdown
print("Converting to Markdown...")
local markdown = roblox.placeToMarkdown(placeContents)

-- Write the result to a file
print("Writing output to: " .. outputPath)
fs.writeFile(outputPath, markdown)

print("Done! The place structure has been written to " .. outputPath) 